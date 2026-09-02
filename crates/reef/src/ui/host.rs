use reef_core::AgentName;
use serde::de::DeserializeOwned;
use std::io::Read;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(60);
const SSH_OPTIONS: [&str; 9] = [
    "-o",
    "BatchMode=yes",
    "-o",
    "ConnectTimeout=10",
    "-o",
    "ServerAliveInterval=5",
    "-o",
    "ServerAliveCountMax=3",
    "-T",
];

#[derive(Clone)]
pub struct Alias(String);

impl Alias {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for Alias {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let token = value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@'));
        if value.is_empty() || value.starts_with('-') || !token {
            return Err(format!("invalid ssh host alias: {value:?}"));
        }
        Ok(Self(value.to_owned()))
    }
}

#[derive(Clone)]
pub enum Host {
    Local { exe: PathBuf, state: PathBuf },
    Ssh { alias: Alias, reef: String },
}

#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    Unreachable(String),
    Misconfigured(String),
    Skew,
    Reef(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(why) => write!(f, "unreachable: {why}"),
            Self::Misconfigured(why) => write!(f, "cannot run reef: {why}"),
            Self::Skew => f.write_str("unexpected output from reef there"),
            Self::Reef(why) => f.write_str(why),
        }
    }
}

impl Host {
    pub fn label(&self) -> &str {
        match self {
            Self::Local { .. } => "local",
            Self::Ssh { alias, .. } => alias.as_str(),
        }
    }

    pub fn fetch<T: DeserializeOwned>(&self, args: &[&str]) -> Result<T, Failure> {
        let output = self.output(args)?;
        serde_json::from_slice(&output).map_err(|_| Failure::Skew)
    }

    pub fn run(&self, args: &[&str]) -> Result<(), Failure> {
        self.output(args).map(drop)
    }

    pub fn terminal(&self, name: &AgentName) -> String {
        match self {
            Self::Local { .. } => format!("reef agent ssh {name}"),
            Self::Ssh { alias, reef } => {
                format!("ssh -t -- {} {reef} agent ssh {name}", alias.as_str())
            }
        }
    }

    pub fn forward(&self, port: u16) -> Option<String> {
        match self {
            Self::Local { .. } => None,
            Self::Ssh { alias, .. } => Some(format!(
                "ssh -N -L {port}:127.0.0.1:{port} -- {}",
                alias.as_str()
            )),
        }
    }

    fn output(&self, args: &[&str]) -> Result<Vec<u8>, Failure> {
        let mut child = self
            .command(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Failure::Unreachable(e.to_string()))?;
        let stdout = drain(child.stdout.take());
        let stderr = drain(child.stderr.take());
        let status = wait(&mut child)?;
        let stderr = message(&stderr.join().unwrap_or_default());
        classify(status, &stderr).map(|()| stdout.join().unwrap_or_default())
    }

    fn command(&self, args: &[&str]) -> Command {
        match self {
            Self::Local { exe, state } => {
                let mut command = Command::new(exe);
                command.arg("--state").arg(state).args(args);
                command
            }
            Self::Ssh { alias, reef } => {
                let mut command = Command::new("ssh");
                command
                    .args(SSH_OPTIONS)
                    .arg("--")
                    .arg(alias.as_str())
                    .arg(format!("{reef} {}", args.join(" ")));
                command
            }
        }
    }
}

fn drain<R: Read + Send + 'static>(reader: Option<R>) -> JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut reader) = reader {
            reader.read_to_end(&mut bytes).ok();
        }
        bytes
    })
}

fn wait(child: &mut Child) -> Result<ExitStatus, Failure> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let waited = child
            .try_wait()
            .map_err(|e| Failure::Unreachable(e.to_string()))?;
        if let Some(status) = waited {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            child.kill().ok();
            child.wait().ok();
            return Err(Failure::Unreachable(format!(
                "no answer in {} s",
                TIMEOUT.as_secs()
            )));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn classify(status: ExitStatus, stderr: &str) -> Result<(), Failure> {
    match status.code() {
        Some(0) => Ok(()),
        Some(255) => Err(Failure::Unreachable(stderr.to_owned())),
        None => Err(Failure::Unreachable(status.signal().map_or_else(
            || stderr.to_owned(),
            |signal| format!("killed by signal {signal}"),
        ))),
        Some(126 | 127) => Err(Failure::Misconfigured(stderr.to_owned())),
        Some(_) if stderr.starts_with("sudo:") => Err(Failure::Misconfigured(stderr.to_owned())),
        Some(_) => Err(Failure::Reef(stderr.to_owned())),
    }
}

fn message(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .find(|line| !line.starts_with("Warning:"))
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(host: &Host) -> Vec<String> {
        let command = host.command(&["agent", "list", "--json"]);
        std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn exited(code: i32) -> ExitStatus {
        ExitStatus::from_raw(code << 8)
    }

    #[test]
    fn aliases_are_tokens() {
        assert!("prod-eu".parse::<Alias>().is_ok());
        assert!("ana@reef-1.internal".parse::<Alias>().is_ok());
        assert!("".parse::<Alias>().is_err());
        assert!("-oProxyCommand=x".parse::<Alias>().is_err());
        assert!("host name".parse::<Alias>().is_err());
        assert!("host;id".parse::<Alias>().is_err());
    }

    #[test]
    fn local_runs_this_binary_against_the_state_dir() {
        let host = Host::Local {
            exe: "/opt/reef".into(),
            state: "/var/reef".into(),
        };
        assert_eq!(
            argv(&host),
            [
                "/opt/reef",
                "--state",
                "/var/reef",
                "agent",
                "list",
                "--json"
            ]
        );
    }

    #[test]
    fn ssh_hosts_get_the_remote_command_after_the_alias() {
        let host = Host::Ssh {
            alias: "prod-eu".parse().unwrap(),
            reef: "sudo -n -u reef -H /home/reef/.local/bin/reef".to_owned(),
        };
        let argv = argv(&host);
        assert_eq!(argv[0], "ssh");
        assert_eq!(
            &argv[argv.len() - 3..],
            [
                "--",
                "prod-eu",
                "sudo -n -u reef -H /home/reef/.local/bin/reef agent list --json"
            ]
        );
        assert!(argv.contains(&"BatchMode=yes".to_owned()));
    }

    #[test]
    fn exit_codes_signals_and_sudo_are_classified() {
        assert_eq!(classify(exited(0), ""), Ok(()));
        assert_eq!(
            classify(exited(255), "ssh: connect to host x: refused"),
            Err(Failure::Unreachable(
                "ssh: connect to host x: refused".to_owned()
            ))
        );
        assert_eq!(
            classify(ExitStatus::from_raw(9), ""),
            Err(Failure::Unreachable("killed by signal 9".to_owned()))
        );
        assert_eq!(
            classify(exited(127), "bash: reef: command not found"),
            Err(Failure::Misconfigured(
                "bash: reef: command not found".to_owned()
            ))
        );
        assert_eq!(
            classify(exited(1), "sudo: a password is required"),
            Err(Failure::Misconfigured(
                "sudo: a password is required".to_owned()
            ))
        );
        assert_eq!(
            classify(exited(1), "Error: no such agent: x"),
            Err(Failure::Reef("Error: no such agent: x".to_owned()))
        );
    }

    #[test]
    fn the_message_is_the_first_line_that_is_not_a_warning() {
        let stderr = b"Warning: Permanently added 'x' (ED25519) to the list of known hosts.\n\
                       Error: cannot read secrets.toml\n\nCaused by:\n    No such file\n";
        assert_eq!(message(stderr), "Error: cannot read secrets.toml");
        assert_eq!(message(b""), "");
    }
}
