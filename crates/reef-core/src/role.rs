use crate::name::{
    Domain, EnvKey, GuestPath, Host, ImageRef, PortName, RoleName, SecretRef, VolumeName,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

const FILES_MAX: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Role {
    pub version: u32,
    pub name: RoleName,
    pub image: ImageRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<EnvKey, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub files: BTreeMap<GuestPath, File>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub expose: BTreeMap<PortName, u16>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub volumes: BTreeMap<VolumeName, Volume>,
    pub resources: Resources,
    pub network: Network,
    #[serde(default)]
    pub secrets: BTreeMap<EnvKey, SecretBinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum File {
    Content(String),
    WithMode { content: String, mode: u32 },
}

impl File {
    pub fn content(&self) -> &str {
        match self {
            Self::Content(content) | Self::WithMode { content, .. } => content,
        }
    }

    pub fn mode(&self) -> Option<u32> {
        match self {
            Self::Content(_) => None,
            Self::WithMode { mode, .. } => Some(*mode),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Volume {
    pub dest: GuestPath,
    pub size_mib: u32,
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
    pub host: Host,
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
        if let Some(init) = &self.init {
            match init.first() {
                None => out.push("init: must name a program".to_owned()),
                Some(cmd) if !cmd.starts_with('/') || cmd.contains('\\') => {
                    out.push(format!("init: {cmd:?} must be an absolute guest path"));
                }
                _ => {}
            }
            if init.iter().any(|part| part.contains('\0')) {
                out.push("init: NUL bytes are not allowed".to_owned());
            }
        }
        for (name, guest) in &self.expose {
            if *guest == 0 {
                out.push(format!("expose.{name}: guest port cannot be 0"));
            }
        }
        for (key, value) in &self.env {
            if key.as_str().starts_with("MSB_") {
                out.push(format!(
                    "env.{key}: the MSB_ prefix is reserved by the runtime"
                ));
            }
            if self.secrets.contains_key(key) {
                out.push(format!("env.{key}: also declared in secrets"));
            }
            if value.contains('\0') {
                out.push(format!("env.{key}: NUL bytes are not allowed"));
            }
        }
        let bytes: usize = self.files.values().map(|file| file.content().len()).sum();
        if bytes > FILES_MAX {
            out.push(format!(
                "files: {bytes} bytes of content, over the {} KiB limit",
                FILES_MAX / 1024
            ));
        }
        for (path, file) in &self.files {
            if let Some((name, _)) = self
                .volumes
                .iter()
                .find(|(_, volume)| path.under(&volume.dest))
            {
                out.push(format!(
                    "files.{path}: volume {name} mounts over it, hiding the file"
                ));
            }
            if file.mode().is_some_and(|mode| mode > 0o777) {
                out.push(format!("files.{path}: mode must be 0o777 or lower"));
            }
        }
        let mut dests = BTreeSet::new();
        for (name, volume) in &self.volumes {
            if volume.size_mib == 0 {
                out.push(format!("volumes.{name}: size-mib must be at least 1"));
            }
            if !dests.insert(&volume.dest) {
                out.push(format!(
                    "volumes.{name}: dest {} is declared twice",
                    volume.dest
                ));
            }
        }
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
        if self.network.egress.len() > 1 && self.network.egress.iter().any(Domain::is_any) {
            out.push(r#"network.egress: "*" allows every host, so it must stand alone"#.to_owned());
        }
        for (key, binding) in &self.secrets {
            if key.as_str().starts_with("MSB_") {
                out.push(format!(
                    "secrets.{key}: the MSB_ prefix is reserved by the runtime"
                ));
            }
            let host = binding.host.as_str();
            if !self.network.egress.iter().any(|rule| rule.covers(host)) {
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

    fn invalid(text: &str) -> Vec<String> {
        match parse_role(text) {
            Err(RoleError::Invalid(problems)) => problems,
            other => panic!("expected invalid, got {other:?}"),
        }
    }

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
    fn volumes_must_be_absolute_sized_and_distinct() {
        let volumes =
            |body: &str| GOOD.replace("[resources]", &format!("[volumes]\n{body}\n[resources]"));

        let role = parse_role(&volumes(
            r#"data = { dest = "/opt/data", size-mib = 10240 }"#,
        ))
        .unwrap();
        assert_eq!(
            role.volumes[&"data".parse::<VolumeName>().unwrap()].size_mib,
            10240
        );

        let err =
            parse_role(&volumes(r#"data = { dest = "opt/data", size-mib = 1 }"#)).unwrap_err();
        assert!(err.to_string().contains("guest path"), "{err}");

        let problems = invalid(&volumes(r#"data = { dest = "/opt/data", size-mib = 0 }"#));
        assert!(problems[0].contains("size-mib"), "{problems:?}");

        let problems = invalid(&volumes(
            "a = { dest = \"/d\", size-mib = 1 }\nb = { dest = \"/d\", size-mib = 1 }",
        ));
        assert!(problems[0].contains("declared twice"), "{problems:?}");
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
        let problems = invalid(&text);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("ANTHROPIC_API_KEY"), "{problems:?}");
        assert!(problems[0].contains("api.openai.com"), "{problems:?}");
    }

    #[test]
    fn secret_hosts_cannot_be_wildcards() {
        let host = |value: &str| GOOD.replace(r#"host = "raw.githubusercontent.com""#, value);

        let err = parse_role(&host(r#"host = "*.githubusercontent.com""#)).unwrap_err();
        assert!(err.to_string().contains("*.githubusercontent.com"), "{err}");

        let err = parse_role(&host(r#"host = "*""#)).unwrap_err();
        assert!(err.to_string().contains('*'), "{err}");
    }

    #[test]
    fn egress_is_required() {
        let text = GOOD.replace("[network]\negress = [\"api.anthropic.com\", \"github.com\", \"*.githubusercontent.com\"]\n\n[secrets]", "[secrets]");
        assert!(parse_role(&text).is_err());
    }

    #[test]
    fn single_label_wildcards_are_rejected() {
        let text = GOOD.replace(r#""*.githubusercontent.com""#, r#""*.internal""#);
        let problems = invalid(&text);
        assert!(problems[0].contains("*.internal"), "{problems:?}");
    }

    #[test]
    fn unrestricted_egress_must_stand_alone() {
        let egress = |list: &str| {
            GOOD.replace(
                r#"egress = ["api.anthropic.com", "github.com", "*.githubusercontent.com"]"#,
                &format!("egress = {list}"),
            )
        };

        let role = parse_role(&egress(r#"["*"]"#)).unwrap();
        assert!(role.network.egress[0].covers("anything.example.com"));
        assert_eq!(role.secrets.len(), 2, "every secret host stays reachable");

        let problems = invalid(&egress(r#"["*", "github.com"]"#));
        assert!(problems[0].contains("stand alone"), "{problems:?}");
    }

    #[test]
    fn init_is_optional_absolute_exec_form() {
        let role = parse_role(GOOD).unwrap();
        assert_eq!(role.init, None);

        let text = GOOD.replace("[resources]", "init = [\"/init\", \"--flag\"]\n[resources]");
        let role = parse_role(&text).unwrap();
        assert_eq!(role.init.unwrap(), ["/init", "--flag"]);

        let text = GOOD.replace("[resources]", "init = [\"nginx\"]\n[resources]");
        assert!(invalid(&text)[0].contains("absolute"));

        let text = GOOD.replace("[resources]", "init = []\n[resources]");
        assert!(invalid(&text)[0].contains("init"));

        let text = GOOD.replace(
            "[resources]",
            "init = [\"/init\", \"a\\u0000b\"]\n[resources]",
        );
        assert!(invalid(&text)[0].contains("NUL"));
    }

    #[test]
    fn env_is_plain_and_disjoint_from_secrets() {
        let role = parse_role(GOOD).unwrap();
        assert!(role.env.is_empty());

        let text = GOOD.replace("[network]", "[env]\nHERMES_DASHBOARD = \"1\"\n\n[network]");
        let role = parse_role(&text).unwrap();
        assert_eq!(role.env.len(), 1);

        let text = GOOD.replace("[network]", "[env]\nMSB_HOME = \"/tmp\"\n\n[network]");
        assert!(invalid(&text)[0].contains("MSB_"));

        let text = GOOD.replace("ANTHROPIC_API_KEY =", "MSB_KEY =");
        assert!(invalid(&text)[0].contains("MSB_"));

        let text = GOOD.replace("[network]", "[env]\nBAD = \"a\\u0000b\"\n\n[network]");
        assert!(invalid(&text)[0].contains("NUL"));

        let text = GOOD.replace("[network]", "[env]\nANTHROPIC_API_KEY = \"x\"\n\n[network]");
        assert!(invalid(&text)[0].contains("secrets"));
    }

    #[test]
    fn files_seed_the_rootfs_and_stay_out_of_volumes() {
        let role = parse_role(GOOD).unwrap();
        assert!(role.files.is_empty());

        let files =
            |body: &str| GOOD.replace("[resources]", &format!("[files]\n{body}\n\n[resources]"));

        let role = parse_role(&files(r#""/etc/agent/config.json" = "{}""#)).unwrap();
        let file = &role.files[&"/etc/agent/config.json".parse().unwrap()];
        assert_eq!((file.content(), file.mode()), ("{}", None));

        let role = parse_role(&files(
            r##""/etc/agent/run" = { content = "#!/bin/sh", mode = 0o755 }"##,
        ))
        .unwrap();
        let file = &role.files[&"/etc/agent/run".parse().unwrap()];
        assert_eq!((file.content(), file.mode()), ("#!/bin/sh", Some(0o755)));

        let problems = invalid(&files(
            r#""/etc/agent/run" = { content = "x", mode = 0o4755 }"#,
        ));
        assert!(problems[0].contains("0o777"), "{problems:?}");

        let err = parse_role(&files(r#""etc/config.json" = "x""#)).unwrap_err();
        assert!(err.to_string().contains("guest path"), "{err}");

        let shadowed = files(
            r#""/root/.gitconfig" = "x"

[volumes]
home = { dest = "/root", size-mib = 1 }"#,
        );
        assert!(invalid(&shadowed)[0].contains("volume home"));

        let big = format!(r#""/etc/big" = "{}""#, "x".repeat(64 * 1024 + 1));
        assert!(invalid(&files(&big))[0].contains("KiB"));
    }

    #[test]
    fn expose_is_named_guest_ports() {
        let role = parse_role(GOOD).unwrap();
        assert!(role.expose.is_empty());

        let text = GOOD.replace(
            "[network]",
            "[expose]\nui = 9119\nterminal = 7681\n\n[network]",
        );
        let role = parse_role(&text).unwrap();
        assert_eq!(role.expose.len(), 2);
        assert_eq!(role.expose[&"ui".parse().unwrap()], 9119);

        let text = GOOD.replace("[network]", "[expose]\nui = 0\n\n[network]");
        assert!(invalid(&text)[0].contains("expose.ui"));
    }

    #[test]
    fn zero_resources_are_named() {
        let text = GOOD.replace("vcpus = 2", "vcpus = 0");
        assert!(invalid(&text)[0].contains("vcpus"));
    }
}
