use std::path::PathBuf;
use std::process::Command;

const ROLE: &str = r#"
version = 1
name = "echo"
image = "alpine"
resources = { vcpus = 1, memory-mib = 64 }
network = { egress = ["example.com"] }
"#;

struct Host(PathBuf);

impl Host {
    fn new(test: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("reef-{test}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("libkrunfw"), "").unwrap();
        let host = Self(dir);
        host.write("role.toml", ROLE);
        host
    }

    fn write(&self, file: &str, text: &str) {
        std::fs::write(self.0.join(file), text).unwrap();
    }

    fn reef(&self, args: &[&str]) -> (bool, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_reef"))
            .current_dir(&self.0)
            .env("REEF_STATE", self.0.join("state"))
            .env("MSB_HOME", self.0.join("msb"))
            .env("MSB_PATH", "/usr/bin/false")
            .env("MSB_LIBKRUNFW_PATH", self.0.join("libkrunfw"))
            .args(args)
            .output()
            .expect("reef binary runs");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        (out.status.success(), stderr)
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn role_apply_pulls_only_what_it_activates_and_a_failed_pull_only_warns() {
    let host = Host::new("role");
    let (ok, stderr) = host.reef(&["role", "apply", "role.toml"]);
    assert!(
        ok && stderr.starts_with("warn   echo: cannot pull alpine"),
        "{stderr}"
    );
    let (ok, stderr) = host.reef(&["role", "apply", "role.toml"]);
    assert!(ok && stderr.is_empty(), "{stderr}");
}

#[test]
fn fleet_apply_converges_new_agents_first_and_prunes_last() {
    let host = Host::new("fleet");
    host.reef(&["role", "apply", "role.toml"]);
    host.write(
        "fleet.toml",
        "version = 1\nagents.a-old.role = \"echo\"\nagents.z-gone.role = \"echo\"\n",
    );
    host.reef(&["fleet", "apply", "fleet.toml"]);
    host.write(
        "fleet.toml",
        "version = 1\nagents.a-old.role = \"echo\"\nagents.b-new.role = \"echo\"\n",
    );
    let (_, stderr) = host.reef(&["fleet", "apply", "fleet.toml"]);
    let order: Vec<_> = stderr
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(name, _)| name)
        .collect();
    assert_eq!(order, ["b-new", "a-old", "z-gone", "Error"], "{stderr}");
}
