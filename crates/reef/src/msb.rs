use crate::vmm::{VmConfig, Vmm};
use anyhow::{Context, Result, bail};
use microsandbox::backend::{Backend, LocalBackend};
use microsandbox::sandbox::{RlimitResource, SandboxHandle, SandboxStatus};
use microsandbox::size::SizeExt;
use microsandbox::{ExecEvent, MicrosandboxError, NetworkPolicy, Sandbox};
use reef_core::VmStatus;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const STATE_LABEL: &str = "reef.state";

pub struct Msb {
    state_id: String,
}

impl Msb {
    pub fn new(state_dir: &Path) -> Result<Self> {
        microsandbox::set_default_backend(Arc::new(LocalBackend::lazy()) as Arc<dyn Backend>);
        let canonical = state_dir
            .canonicalize()
            .unwrap_or_else(|_| state_dir.to_owned());
        let hash = Sha256::digest(canonical.as_os_str().as_encoded_bytes());
        let state_id = hash.iter().take(4).map(|b| format!("{b:02x}")).collect();
        Ok(Self { state_id })
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

    async fn create(&self, config: VmConfig) -> Result<()> {
        let mut builder = Sandbox::builder(&config.name)
            .image(config.image.as_str())
            .cpus(config.vcpus)
            .memory(config.memory_mib)
            .label(STATE_LABEL, &self.state_id)
            .replace();
        if let Some(gib) = config.disk_gib {
            builder = builder.root_disk(gib.gib());
        }
        if let Some(pids) = config.max_pids {
            builder = builder.rlimit(RlimitResource::Nproc, u64::from(pids));
        }
        if let Some(mount) = &config.volume {
            let volume = mount.volume.clone();
            builder = builder.volume(&mount.dest, |m| m.named_with(volume, |n| n.ensure_exists()));
        }
        let policy = egress_policy(&config.egress)?;
        builder = builder.network(|n| n.policy(policy));
        for secret in &config.secrets {
            builder = builder.secret(|s| {
                let s = s.env(&secret.key).value(secret.value.expose());
                match secret.host.strip_prefix("*.") {
                    Some(_) => s.allow_host_pattern(&secret.host),
                    None => s.allow_host(&secret.host),
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

    async fn exec(&self, name: &str, command: &[String]) -> Result<i32> {
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
}

fn egress_policy(domains: &[String]) -> Result<NetworkPolicy> {
    let mut exact = Vec::new();
    let mut suffixes = Vec::new();
    for domain in domains {
        match domain.strip_prefix("*.") {
            Some(suffix) => suffixes.push(suffix.to_owned()),
            None => exact.push(domain.clone()),
        }
    }
    NetworkPolicy::builder()
        .default_deny()
        .egress(move |mut rule| {
            if !exact.is_empty() {
                rule = rule.allow_domains(exact);
            }
            if !suffixes.is_empty() {
                rule = rule.allow_domain_suffixes(suffixes);
            }
            rule
        })
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
    if !std::path::Path::new("/dev/kvm").exists() {
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
