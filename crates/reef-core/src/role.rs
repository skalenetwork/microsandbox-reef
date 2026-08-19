use crate::name::{Domain, EnvKey, ImageRef, RoleName, SecretRef};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Role {
    pub version: u32,
    pub name: RoleName,
    pub image: ImageRef,
    pub resources: Resources,
    pub network: Network,
    #[serde(default)]
    pub secrets: BTreeMap<EnvKey, SecretBinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Resources {
    pub vcpus: u8,
    pub memory_mib: u32,
    #[serde(default)]
    pub disk_gib: Option<u32>,
    #[serde(default)]
    pub max_pids: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Network {
    pub egress: Vec<Domain>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretBinding {
    #[serde(rename = "ref")]
    pub secret: SecretRef,
    pub host: Domain,
}

#[derive(Debug)]
pub enum RoleError {
    Toml(toml::de::Error),
    Version(u32),
    Invalid(Vec<String>),
}

impl fmt::Display for RoleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toml(e) => e.fmt(f),
            Self::Version(v) => {
                write!(
                    f,
                    "unsupported role version {v} (this reef reads version 1)"
                )
            }
            Self::Invalid(problems) => f.write_str(&problems.join("\n")),
        }
    }
}

impl std::error::Error for RoleError {}

#[derive(Deserialize)]
struct VersionPeek {
    version: u32,
}

pub fn parse_role(text: &str) -> Result<Role, RoleError> {
    let peek: VersionPeek = toml::from_str(text).map_err(RoleError::Toml)?;
    if peek.version != 1 {
        return Err(RoleError::Version(peek.version));
    }
    let role: Role = toml::from_str(text).map_err(RoleError::Toml)?;
    let problems = role.problems();
    if problems.is_empty() {
        Ok(role)
    } else {
        Err(RoleError::Invalid(problems))
    }
}

impl Role {
    fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.resources.vcpus == 0 {
            out.push("resources.vcpus: must be at least 1".to_owned());
        }
        if self.resources.memory_mib < 64 {
            out.push("resources.memory-mib: must be at least 64".to_owned());
        }
        if self.resources.disk_gib == Some(0) {
            out.push("resources.disk-gib: must be at least 1".to_owned());
        }
        if self.resources.max_pids == Some(0) {
            out.push("resources.max-pids: must be at least 1".to_owned());
        }
        for rule in &self.network.egress {
            if let Some(suffix) = rule.wildcard_suffix()
                && !suffix.contains('.')
            {
                out.push(format!(
                    "network.egress: {rule} is a single-label wildcard, which the runtime rejects"
                ));
            }
        }
        for (key, binding) in &self.secrets {
            let host = binding.host.as_str();
            let reachable = self
                .network
                .egress
                .iter()
                .any(|rule| rule.as_str() == host || rule.covers(host));
            if !reachable {
                out.push(format!(
                    "secrets.{key}: host {host} is not covered by network.egress"
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
version = 1
name  = "code-reviewer"
image = "ghcr.io/acme/agent@sha256:9f2c000000000000000000000000000000000000000000000000000000000000"

[resources]
vcpus = 2
memory-mib = 4096
disk-gib = 20
max-pids = 512

[network]
egress = ["api.anthropic.com", "github.com", "*.githubusercontent.com"]

[secrets]
ANTHROPIC_API_KEY = { ref = "reef://platform/anthropic", host = "api.anthropic.com" }
RAW_TOKEN         = { ref = "reef://platform/raw", host = "raw.githubusercontent.com" }
"#;

    #[test]
    fn parses_the_strawman() {
        let role = parse_role(GOOD).unwrap();
        assert_eq!(role.name.as_str(), "code-reviewer");
        assert_eq!(role.resources.vcpus, 2);
        assert_eq!(role.network.egress.len(), 3);
        assert_eq!(role.secrets.len(), 2);
    }

    #[test]
    fn rejects_unknown_fields_with_position() {
        let text = GOOD.replace("[resources]", "surprise = true\n[resources]");
        let err = parse_role(&text).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("surprise"), "{msg}");
        assert!(msg.contains("line"), "{msg}");
    }

    #[test]
    fn rejects_future_versions_before_shape_errors() {
        let text = GOOD.replace("version = 1", "version = 2\nnew-field = true");
        match parse_role(&text) {
            Err(RoleError::Version(2)) => {}
            other => panic!("expected version error, got {other:?}"),
        }
    }

    #[test]
    fn secret_host_must_be_reachable() {
        let text = GOOD.replace(
            r#"host = "api.anthropic.com""#,
            r#"host = "api.openai.com""#,
        );
        match parse_role(&text) {
            Err(RoleError::Invalid(problems)) => {
                assert_eq!(problems.len(), 1);
                assert!(problems[0].contains("ANTHROPIC_API_KEY"), "{problems:?}");
                assert!(problems[0].contains("api.openai.com"), "{problems:?}");
            }
            other => panic!("expected invalid, got {other:?}"),
        }
    }

    #[test]
    fn wildcard_egress_covers_secret_host() {
        let role = parse_role(GOOD).unwrap();
        assert!(
            role.secrets
                .contains_key(&EnvKey::try_from("RAW_TOKEN".to_owned()).unwrap())
        );
    }

    #[test]
    fn egress_is_required() {
        let text = GOOD.replace("[network]\negress = [\"api.anthropic.com\", \"github.com\", \"*.githubusercontent.com\"]\n\n[secrets]", "[secrets]");
        assert!(parse_role(&text).is_err());
    }

    #[test]
    fn single_label_wildcards_are_rejected() {
        let text = GOOD.replace(r#""*.githubusercontent.com""#, r#""*.internal""#);
        match parse_role(&text) {
            Err(RoleError::Invalid(problems)) => {
                assert!(problems[0].contains("*.internal"), "{problems:?}")
            }
            other => panic!("expected invalid, got {other:?}"),
        }
    }

    #[test]
    fn zero_resources_are_named() {
        let text = GOOD.replace("vcpus = 2", "vcpus = 0");
        match parse_role(&text) {
            Err(RoleError::Invalid(problems)) => {
                assert!(problems[0].contains("vcpus"), "{problems:?}")
            }
            other => panic!("expected invalid, got {other:?}"),
        }
    }
}
