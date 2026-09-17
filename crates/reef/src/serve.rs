use crate::msb::Msb;
use crate::reconcile;
use crate::store::Store;
use crate::vmm::Vmm;
use anyhow::{Context, Result, bail};
use reef_core::AgentName;
use ssh_key::{Certificate, PublicKey};

pub async fn run(store: &Store, msb: &Msb) -> Result<()> {
    let name = requested()?;
    let auth = std::env::var("SSH_USER_AUTH")
        .context("SSH_USER_AUTH is not set; sshd needs `ExposeAuthInfo yes`")?;
    let auth = std::fs::read_to_string(&auth).with_context(|| format!("cannot read {auth}"))?;
    let cert = certificate(&auth).context(
        "this session did not authenticate with a certificate; \
         the client needs one signed by the trusted CA",
    )?;
    let agent = store
        .get_agent(&name)?
        .with_context(|| format!("no such agent: {name}"))?;
    if !cert.valid_principals().contains(&agent.spec.owner) {
        bail!("access denied: this certificate cannot open {name}");
    }
    let sandbox = reconcile::sandbox_name(&name);
    if !agent.spec.desired.live(msb.status(&sandbox).await?) {
        store.record(&name, "refused", &agent.spec.owner)?;
        bail!("{name} is not running; it has to be started on its host first");
    }
    store.record(&name, "served", &agent.spec.owner)?;
    let key = PublicKey::from(cert.public_key().clone()).to_openssh()?;
    msb.serve(&sandbox, &key).await
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

fn certificate(auth: &str) -> Option<Certificate> {
    auth.lines()
        .filter_map(|line| line.strip_prefix("publickey "))
        .find_map(|key| Certificate::from_openssh(key).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn the_certificate_names_its_principals_and_the_holders_key() {
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
        let key = std::fs::read_to_string(dir.join("id.pub")).unwrap();
        let cert = std::fs::read_to_string(dir.join("id-cert.pub")).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        let cert = certificate(&format!("publickey {key}publickey {cert}")).unwrap();
        assert_eq!(cert.valid_principals(), ["ana", "ops"]);
        assert_eq!(
            cert.public_key(),
            PublicKey::from_openssh(&key).unwrap().key_data()
        );
        assert!(certificate(&format!("publickey {key}")).is_none());
        assert!(certificate("password\n").is_none());
    }
}
