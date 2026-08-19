use crate::vmm::{VmConfig, Vmm};
use anyhow::{Context, Result, bail};
use microsandbox::backend::LocalBackend;
use microsandbox::protocol::message::MessageType;
use microsandbox::protocol::tcp::{TcpClose, TcpConnect, TcpConnected, TcpData, TcpEof, TcpFailed};
use microsandbox::sandbox::{RlimitResource, SandboxHandle, SandboxStatus};
use microsandbox::size::SizeExt;
use microsandbox::{AgentClient, ExecEvent, MicrosandboxError, NetworkPolicy, Sandbox};
use reef_core::{Domain, VmStatus};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const STATE_LABEL: &str = "reef.state";

pub struct Msb {
    state_id: String,
}

impl Msb {
    pub fn new(state_dir: &Path) -> Self {
        microsandbox::set_default_backend(LocalBackend::lazy());
        let canonical = state_dir
            .canonicalize()
            .unwrap_or_else(|_| state_dir.to_owned());
        let hash = Sha256::digest(canonical.as_os_str().as_encoded_bytes());
        let state_id = hash.iter().take(4).map(|b| format!("{b:02x}")).collect();
        Self { state_id }
    }

    fn owned(&self, handle: &SandboxHandle) -> Result<bool> {
        let config = handle.config()?;
        Ok(config
            .spec
            .labels
            .iter()
            .any(|(key, value)| key == STATE_LABEL && *value == self.state_id))
    }
}

impl Vmm for Msb {
    async fn status(&self, name: &str) -> Result<Option<VmStatus>> {
        match Sandbox::get(name).await {
            Ok(handle) => Ok(Some(map_status(handle.status_snapshot()))),
            Err(e) if is_not_found(&e) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn create(&self, config: VmConfig<'_>) -> Result<()> {
        let role = config.role;
        let mut builder = Sandbox::builder(&config.name)
            .image(role.image.as_str())
            .cpus(role.resources.vcpus)
            .memory(role.resources.memory_mib)
            .label(STATE_LABEL, &self.state_id)
            .replace();
        if let Some((cmd, args)) = role.init.as_deref().and_then(<[String]>::split_first) {
            builder = builder.init_with(cmd, |init| init.args(args));
        }
        for (key, value) in &role.env {
            builder = builder.env(key.as_str(), value);
        }
        if let Some(gib) = role.resources.disk_gib {
            builder = builder.root_disk(gib.gib());
        }
        if let Some(pids) = role.resources.max_pids {
            builder = builder.rlimit(RlimitResource::Nproc, u64::from(pids));
        }
        if let Some(mount) = &config.volume {
            let volume = mount.volume.clone();
            builder = builder.volume(&mount.dest, |m| m.named_with(volume, |n| n.ensure_exists()));
        }
        let policy = egress_policy(&role.network.egress)?;
        builder = builder.network(|n| n.policy(policy));
        for secret in &config.secrets {
            builder = builder.secret(|s| {
                let s = s.env(secret.key.as_str()).value(secret.value.expose());
                match secret.host.wildcard_suffix() {
                    Some(_) => s.allow_host_pattern(secret.host.as_str()),
                    None => s.allow_host(secret.host.as_str()),
                }
            });
        }
        builder.create_detached().await?;
        Ok(())
    }

    async fn start(&self, name: &str) -> Result<()> {
        Sandbox::start_detached(name).await?;
        Ok(())
    }

    async fn stop(&self, name: &str) -> Result<()> {
        Sandbox::get(name).await?.stop().await?;
        Ok(())
    }

    async fn remove(&self, name: &str) -> Result<()> {
        let handle = match Sandbox::get(name).await {
            Ok(handle) => handle,
            Err(e) if is_not_found(&e) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        if !self.owned(&handle)? {
            bail!(
                "sandbox {name} exists but was not created by this reef state dir; \
                 refusing to destroy it (remove it with `msb rm` if it is really yours)"
            );
        }
        if map_status(handle.status_snapshot()) == VmStatus::Running {
            handle.stop().await?;
        }
        Sandbox::remove(name).await?;
        Ok(())
    }
}

impl Msb {
    pub async fn exec(&self, name: &str, command: &[String]) -> Result<i32> {
        let (cmd, args) = command.split_first().context("empty command")?;
        let sandbox = Sandbox::get(name)
            .await?
            .connect()
            .await
            .context("agent VM is not running")?;
        let mut handle = sandbox.exec_stream(cmd, args).await?;
        loop {
            match handle.recv().await {
                Some(ExecEvent::Stdout(bytes)) => {
                    let mut out = std::io::stdout().lock();
                    out.write_all(&bytes)?;
                    out.flush()?;
                }
                Some(ExecEvent::Stderr(bytes)) => {
                    let mut err = std::io::stderr().lock();
                    err.write_all(&bytes)?;
                    err.flush()?;
                }
                Some(ExecEvent::Exited { code }) => return Ok(code),
                Some(ExecEvent::Failed(failure)) => bail!("exec failed to start: {failure:?}"),
                Some(_) => {}
                None => bail!("exec stream ended without an exit code"),
            }
        }
    }

    pub async fn forward(&self, name: &str, ports: &[(u16, u16)]) -> Result<()> {
        let sandbox = Sandbox::get(name)
            .await?
            .connect()
            .await
            .context("agent VM is not running")?;
        let client = sandbox.client_arc();
        if !client.supports(MessageType::TcpConnect) {
            bail!(
                "this VM's runtime predates port forwarding; restart the agent (stop, then start)"
            );
        }
        let mut serve = tokio::task::JoinSet::new();
        for &(local, guest) in ports {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", local))
                .await
                .with_context(|| format!("cannot bind 127.0.0.1:{local}"))?;
            println!(
                "forwarding http://{} -> {name}:{guest}",
                listener.local_addr()?
            );
            let client = client.clone();
            serve.spawn(async move {
                loop {
                    match listener.accept().await {
                        Ok((socket, _)) => {
                            let client = client.clone();
                            tokio::spawn(async move {
                                if let Err(e) = tunnel(client, socket, guest).await {
                                    eprintln!("forward to :{guest}: {e:#}");
                                }
                            });
                        }
                        Err(e) => {
                            eprintln!("accept for :{guest}: {e}");
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                    }
                }
            });
        }
        let vanished = async {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                if matches!(
                    self.status(name).await,
                    Ok(None) | Ok(Some(VmStatus::Stopped))
                ) {
                    return;
                }
            }
        };
        tokio::select! {
            () = vanished => bail!("agent VM is no longer running"),
            _ = serve.join_all() => Ok(()),
        }
    }
}

async fn tunnel(client: Arc<AgentClient>, socket: tokio::net::TcpStream, guest: u16) -> Result<()> {
    let connect = TcpConnect {
        host: "127.0.0.1".to_owned(),
        port: guest,
    };
    let (id, mut rx) = client.stream(MessageType::TcpConnect, &connect).await?;
    match rx.recv().await {
        Some(msg) if msg.t == MessageType::TcpConnected => {
            let _: TcpConnected = msg.payload()?;
        }
        Some(msg) if msg.t == MessageType::TcpFailed => {
            let failed: TcpFailed = msg.payload()?;
            bail!("guest refused :{guest}: {}", failed.error);
        }
        _ => bail!("agent closed the tunnel before it connected"),
    }
    let (mut local_read, mut local_write) = socket.into_split();
    let outbound = tokio::spawn({
        let client = client.clone();
        async move {
            let mut buf = vec![0u8; 32 * 1024];
            loop {
                match local_read.read(&mut buf).await {
                    Ok(0) => {
                        let _ = client.send(id, MessageType::TcpEof, &TcpEof {}).await;
                        return;
                    }
                    Err(_) => {
                        let _ = client.send(id, MessageType::TcpClose, &TcpClose {}).await;
                        return;
                    }
                    Ok(n) => {
                        let data = TcpData {
                            data: buf[..n].to_vec(),
                        };
                        if client.send(id, MessageType::TcpData, &data).await.is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });
    let inbound = async {
        while let Some(msg) = rx.recv().await {
            match msg.t {
                MessageType::TcpData => {
                    let data: TcpData = msg.payload()?;
                    local_write.write_all(&data.data).await?;
                }
                MessageType::TcpEof => local_write.shutdown().await?,
                MessageType::TcpClosed => break,
                MessageType::TcpFailed => {
                    let failed: TcpFailed = msg.payload()?;
                    eprintln!("forward to :{guest}: guest side failed: {}", failed.error);
                    break;
                }
                _ => {}
            }
        }
        anyhow::Ok(())
    };
    let result = inbound.await;
    outbound.abort();
    let _ = client.send(id, MessageType::TcpClose, &TcpClose {}).await;
    result
}

fn egress_policy(domains: &[Domain]) -> Result<NetworkPolicy> {
    let mut exact = Vec::new();
    let mut suffixes = Vec::new();
    for domain in domains {
        match domain.wildcard_suffix() {
            Some(suffix) => suffixes.push(suffix),
            None => exact.push(domain.as_str()),
        }
    }
    NetworkPolicy::builder()
        .default_deny()
        .egress(move |rule| rule.allow_domains(exact).allow_domain_suffixes(suffixes))
        .build()
        .context("egress policy")
}

fn map_status(status: SandboxStatus) -> VmStatus {
    match status {
        SandboxStatus::Running | SandboxStatus::Starting | SandboxStatus::Draining => {
            VmStatus::Running
        }
        SandboxStatus::Created
        | SandboxStatus::Paused
        | SandboxStatus::Stopped
        | SandboxStatus::Crashed => VmStatus::Stopped,
    }
}

fn is_not_found(error: &MicrosandboxError) -> bool {
    matches!(error, MicrosandboxError::SandboxNotFound(_))
}

pub fn doctor() -> Result<()> {
    let msb = microsandbox::config::resolve_msb_path().context(
        "msb not found: set MSB_PATH or install microsandbox (https://microsandbox.dev)",
    )?;
    let version = std::process::Command::new(&msb).arg("--version").output()?;
    if !version.status.success() {
        bail!("{} exists but `--version` failed", msb.display());
    }
    println!(
        "msb    {} ({})",
        String::from_utf8_lossy(&version.stdout).trim(),
        msb.display()
    );
    #[cfg(target_os = "linux")]
    if !Path::new("/dev/kvm").exists() {
        bail!("/dev/kvm is missing: this host cannot run microVMs");
    }
    let home = std::env::var("MSB_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".microsandbox")))?;
    println!("state  {}", home.display());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(meta) = std::fs::metadata(&home)
            && meta.mode() & 0o077 != 0
        {
            println!(
                "warn   {} is readable by other users; sandbox configs there hold secret values - chmod 700 it",
                home.display()
            );
        }
    }
    Ok(())
}
