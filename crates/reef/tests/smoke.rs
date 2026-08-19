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
resources = { vcpus = 1, memory-mib = 256, max-pids = 128 }
network = { egress = ["example.com"] }
secrets = { FAKE_KEY = { ref = "reef://demo/fake", host = "example.com" } }
"#,
    )
    .unwrap();

    let agent = format!("smoke-{}", std::process::id());
    let reef = Reef {
        state,
        agent: agent.clone(),
    };

    let applied = reef.ok(&["role", "apply", role.to_str().unwrap()]);
    assert!(applied.contains("(active)"), "{applied}");

    let created = reef.ok(&["agent", "create", "--role", "echo", "--name", &agent]);
    assert!(created.contains("running"), "{created}");

    let secret = reef.ok(&[
        "agent",
        "exec",
        &agent,
        "--",
        "sh",
        "-c",
        "echo [$FAKE_KEY]",
    ]);
    assert!(secret.contains("[$MSB_FAKE_KEY]"), "value leaked: {secret}");

    let denied = reef.ok(&[
        "agent",
        "exec",
        &agent,
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
        &agent,
        "--",
        "sh",
        "-c",
        "echo keep > /root/marker",
    ]);
    reef.ok(&["agent", "stop", &agent]);
    reef.ok(&["agent", "start", &agent]);
    let marker = reef.ok(&["agent", "exec", &agent, "--", "cat", "/root/marker"]);
    assert!(
        marker.contains("keep"),
        "rootfs lost on stop/start: {marker}"
    );

    let history = reef.ok(&["agent", "history", &agent]);
    assert_eq!(
        history.matches(" create ").count(),
        1,
        "stop/start must not recreate:\n{history}"
    );

    let removed = reef.ok(&["agent", "rm", &agent]);
    assert!(removed.contains("removed"), "{removed}");
}
