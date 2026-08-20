use std::path::PathBuf;
use std::process::Command;

struct Reef {
    state: PathBuf,
    agent: String,
}

impl Reef {
    fn run(&self, args: &[&str]) -> (bool, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_reef"))
            .arg("--state")
            .arg(&self.state)
            .args(args)
            .output()
            .expect("reef binary runs");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.success(), text)
    }

    fn ok(&self, args: &[&str]) -> String {
        let (success, text) = self.run(args);
        assert!(success, "reef {args:?} failed:\n{text}");
        text
    }
}

impl Drop for Reef {
    fn drop(&mut self) {
        let _ = self.run(&["agent", "rm", &self.agent]);
    }
}

fn web_port(json: &str) -> u16 {
    json.split("\"web\": ")
        .nth(1)
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|port| port.parse().ok())
        .unwrap_or_else(|| panic!("no web port in {json:?}"))
}

fn read_banner(port: u16) -> String {
    use std::io::Read;
    let mut response = String::new();
    for _ in 0..20 {
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_millis(500)))
            .unwrap();
        let mut buf = [0u8; 64];
        if let Ok(n) = stream.read(&mut buf) {
            response = String::from_utf8_lossy(&buf[..n]).into_owned();
        }
        if response.contains("reef-forward") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    response
}

#[test]
#[ignore = "boots a real microVM; needs msb and KVM/HVF"]
fn full_agent_journey() {
    let state = std::env::temp_dir().join(format!("reef-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(
        state.join("secrets.toml"),
        "[demo]\nfake = \"sk-smoke-not-real\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            state.join("secrets.toml"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    let role = state.join("echo.toml");
    std::fs::write(
        &role,
        r#"
version = 1
name  = "echo"
image = "alpine"
init  = ["/bin/sleep", "999999999"]
env = { SMOKE_MARK = "reef-env" }
expose = { web = 8080 }
resources = { vcpus = 1, memory-mib = 256, max-pids = 128 }
network = { egress = ["example.com"] }
secrets = { FAKE_KEY = { ref = "reef://demo/fake", host = "example.com" } }
"#,
    )
    .unwrap();

    let reef = Reef {
        state,
        agent: format!("smoke-{}", std::process::id()),
    };

    let applied = reef.ok(&["role", "apply", role.to_str().unwrap()]);
    assert!(applied.contains("(active)"), "{applied}");

    let created = reef.ok(&[
        "agent",
        "create",
        "--role",
        "echo",
        "--name",
        &reef.agent,
        "--env",
        "SMOKE_MARK=agent-wins",
    ]);
    assert!(created.contains("running"), "{created}");

    let got = reef.ok(&["agent", "get", &reef.agent, "--wait", "--json"]);
    assert!(got.contains(r#""state": "running""#), "{got}");
    assert!(got.contains(r#""vm": "running""#), "{got}");
    let web = web_port(&got);
    assert!(reef_core::HOST_PORTS.contains(&web), "{got}");

    let pid1 = reef.ok(&["agent", "exec", &reef.agent, "--", "cat", "/proc/1/comm"]);
    assert!(
        pid1.contains("sleep"),
        "init handoff did not happen: {pid1}"
    );

    let secret = reef.ok(&[
        "agent",
        "exec",
        &reef.agent,
        "--",
        "sh",
        "-c",
        "echo [$FAKE_KEY] $SMOKE_MARK",
    ]);
    assert!(secret.contains("[$MSB_FAKE_KEY]"), "value leaked: {secret}");
    assert!(
        secret.contains("agent-wins"),
        "agent env must override role env: {secret}"
    );

    let denied = reef.ok(&[
        "agent",
        "exec",
        &reef.agent,
        "--",
        "sh",
        "-c",
        "wget -T 3 -qO- https://api.openai.com/ 2>&1 || true",
    ]);
    assert!(
        denied.contains("bad address"),
        "egress not denied: {denied}"
    );

    reef.ok(&[
        "agent",
        "exec",
        &reef.agent,
        "--",
        "sh",
        "-c",
        "echo keep > /root/marker",
    ]);
    reef.ok(&["agent", "stop", &reef.agent]);
    reef.ok(&["agent", "start", &reef.agent]);
    let marker = reef.ok(&["agent", "exec", &reef.agent, "--", "cat", "/root/marker"]);
    assert!(
        marker.contains("keep"),
        "rootfs lost on stop/start: {marker}"
    );
    let pid1 = reef.ok(&["agent", "exec", &reef.agent, "--", "cat", "/proc/1/comm"]);
    assert!(
        pid1.contains("sleep"),
        "init lost across stop/start: {pid1}"
    );

    reef.ok(&[
        "agent",
        "exec",
        &reef.agent,
        "--",
        "sh",
        "-c",
        "(while true; do echo reef-forward | nc -l -p 8080; done >/dev/null 2>&1 &)",
    ]);
    let listening = reef.ok(&["agent", "forward", &reef.agent]);
    assert!(listening.contains("8080"), "{listening}");

    struct Kill(std::process::Child);
    impl Drop for Kill {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut fwd = Kill(
        Command::new(env!("CARGO_BIN_EXE_reef"))
            .arg("--state")
            .arg(&reef.state)
            .args(["agent", "forward", &reef.agent, "0:8080"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let port = {
        use std::io::BufRead;
        let mut line = String::new();
        std::io::BufReader::new(fwd.0.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        line.split(':')
            .nth(2)
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|port| port.parse::<u16>().ok())
            .unwrap_or_else(|| panic!("no forwarded port in {line:?}"))
    };
    let response = read_banner(port);
    drop(fwd);
    assert!(response.contains("reef-forward"), "{response}");

    let published = read_banner(web);
    assert!(
        published.contains("reef-forward"),
        "published port {web} did not reach the guest: {published}"
    );

    let history = reef.ok(&["agent", "history", &reef.agent]);
    assert_eq!(
        history.matches(" create ").count(),
        1,
        "stop/start must not recreate:\n{history}"
    );

    let removed = reef.ok(&["agent", "rm", &reef.agent]);
    assert!(removed.contains("removed"), "{removed}");

    let broken_role = reef.state.join("broken.toml");
    std::fs::write(
        &broken_role,
        r#"
version = 1
name  = "broken"
image = "reef-smoke-no-such-image"
resources = { vcpus = 1, memory-mib = 256 }
network = { egress = ["example.com"] }
"#,
    )
    .unwrap();
    reef.ok(&["role", "apply", broken_role.to_str().unwrap()]);
    let broken = Reef {
        state: reef.state.clone(),
        agent: format!("broken-{}", std::process::id()),
    };
    let (created, _) = broken.run(&[
        "agent",
        "create",
        "--role",
        "broken",
        "--name",
        &broken.agent,
    ]);
    assert!(!created, "create with a missing image must fail");
    let (settled, got) = broken.run(&["agent", "get", &broken.agent, "--wait", "--json"]);
    assert!(!settled, "get --wait on a failed agent must exit nonzero");
    assert!(got.contains(r#""state": "failed""#), "{got}");
    assert!(got.contains(r#""reason""#), "{got}");
}
