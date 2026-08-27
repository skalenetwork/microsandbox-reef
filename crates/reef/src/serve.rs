use crate::store::Store;
use crate::{msb, reconcile};
use anyhow::{Context, Result, bail};
use reef_core::AgentName;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

pub fn run(store: &Store) -> Result<()> {
    let name = requested()?;
    let auth = std::env::var("SSH_USER_AUTH")
        .context("SSH_USER_AUTH is not set; sshd needs `ExposeAuthInfo yes`")?;
    let auth = std::fs::read_to_string(&auth).with_context(|| format!("cannot read {auth}"))?;
    let principals = certificate_principals(&auth)?;
    let agent = store
        .get_agent(&name)?
        .with_context(|| format!("no such agent: {name}"))?;
    if !principals.contains(&agent.spec.owner) {
        bail!("access denied: this certificate cannot open {name}");
    }
    store.record(&name, "served", &agent.spec.owner)?;
    let error = Command::new(msb::msb_path()?)
        .args(["ssh", "serve", "--stdio", &reconcile::sandbox_name(&name)])
        .exec();
    Err(error).context("cannot run msb ssh serve")
}

fn requested() -> Result<AgentName> {
    let command = std::env::var("SSH_ORIGINAL_COMMAND").context(
        "SSH_ORIGINAL_COMMAND is not set; serve runs as an sshd ForceCommand \
         and the ssh client names the agent to open",
    )?;
    command
        .parse()
        .map_err(|e| anyhow::anyhow!("SSH_ORIGINAL_COMMAND: {e}"))
}

fn certificate_principals(auth: &str) -> Result<Vec<String>> {
    let cert = certificate(auth).context(
        "this session did not authenticate with a certificate; \
         the client needs one signed by the trusted CA",
    )?;
    let mut child = Command::new("ssh-keygen")
        .args(["-L", "-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("cannot run ssh-keygen")?;
    child.stdin.take().unwrap().write_all(cert.as_bytes())?;
    let listing = child.wait_with_output()?;
    if !listing.status.success() {
        bail!(
            "ssh-keygen rejected the certificate: {}",
            String::from_utf8_lossy(&listing.stderr).trim()
        );
    }
    Ok(principals(&String::from_utf8_lossy(&listing.stdout)))
}

fn certificate(auth: &str) -> Option<&str> {
    auth.lines()
        .filter_map(|line| line.strip_prefix("publickey "))
        .find(|key| {
            key.split_whitespace()
                .next()
                .is_some_and(|algorithm| algorithm.ends_with("-cert-v01@openssh.com"))
        })
}

fn principals(listing: &str) -> Vec<String> {
    let mut lines = listing.lines();
    let Some(header) = lines.find(|line| line.trim() == "Principals:") else {
        return Vec::new();
    };
    let depth = indent(header);
    lines
        .take_while(|line| !line.trim().is_empty() && indent(line) > depth)
        .map(|line| line.trim().to_owned())
        .collect()
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principals_come_from_their_indented_block() {
        let listing = "id-cert.pub:\n        \
            Type: ssh-ed25519-cert-v01@openssh.com user certificate\n        \
            Key ID: \"ana-cert\"\n        \
            Principals: \n                \
            ana\n                \
            hermes-ops\n        \
            Critical Options: (none)\n        \
            Extensions: \n                \
            permit-pty\n";
        assert_eq!(principals(listing), ["ana", "hermes-ops"]);
        assert!(principals("        Principals: (none)\n").is_empty());
        assert!(principals("").is_empty());
    }

    #[test]
    fn only_certificate_lines_are_certificates() {
        let auth = "publickey ssh-ed25519 AAAAC3Nza key\n\
                    publickey ssh-ed25519-cert-v01@openssh.com AAAA1234\n";
        assert_eq!(
            certificate(auth),
            Some("ssh-ed25519-cert-v01@openssh.com AAAA1234")
        );
        assert_eq!(certificate("publickey ssh-ed25519 AAAAC3Nza\n"), None);
        assert_eq!(certificate("password\n"), None);
        assert_eq!(certificate(""), None);
    }

    #[test]
    fn a_real_certificate_yields_its_principals() {
        let dir = std::env::temp_dir().join(format!("reef-serve-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let keygen = |args: &[&str]| {
            let status = Command::new("ssh-keygen")
                .args(args)
                .current_dir(&dir)
                .status()
                .unwrap();
            assert!(status.success());
        };
        keygen(&["-q", "-t", "ed25519", "-N", "", "-f", "ca"]);
        keygen(&["-q", "-t", "ed25519", "-N", "", "-f", "id"]);
        keygen(&["-q", "-s", "ca", "-I", "test", "-n", "ana,ops", "id.pub"]);
        let cert = std::fs::read_to_string(dir.join("id-cert.pub")).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        let auth = format!("publickey {cert}");
        assert_eq!(certificate_principals(&auth).unwrap(), ["ana", "ops"]);
        assert!(
            certificate_principals("publickey ssh-ed25519 AAAAC3Nza\n")
                .unwrap_err()
                .to_string()
                .contains("certificate")
        );
    }
}
