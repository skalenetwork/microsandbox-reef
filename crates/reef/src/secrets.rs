use anyhow::{Context, Result, bail};
use reef_core::SecretRef;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(redacted)")
    }
}

#[derive(Deserialize)]
#[serde(try_from = "String")]
struct CommandTemplate(String);

impl TryFrom<String> for CommandTemplate {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.trim().is_empty() {
            return Err("resolver command is empty".to_owned());
        }
        if !value.contains("{name}") {
            return Err(format!("resolver command must contain {{name}}: {value:?}"));
        }
        Ok(Self(value))
    }
}

pub struct Secrets {
    path: PathBuf,
    resolvers: BTreeMap<String, CommandTemplate>,
    stores: BTreeMap<String, BTreeMap<String, String>>,
}

impl Secrets {
    pub fn load(path: &Path) -> Result<Self> {
        let (resolvers, stores) = match std::fs::metadata(path) {
            Ok(meta) => {
                require_private(path, &meta)?;
                let text = std::fs::read_to_string(path)
                    .with_context(|| format!("cannot read {}", path.display()))?;
                parse(&text).map_err(|message| {
                    anyhow::anyhow!(
                        "cannot parse {}: {message} (the offending line is not shown because the file holds secrets)",
                        path.display()
                    )
                })?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Default::default(),
            Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
        };
        Ok(Self {
            path: path.to_owned(),
            resolvers,
            stores,
        })
    }

    pub fn resolve(&self, secret: &SecretRef) -> Result<Secret> {
        if let Some(value) = self
            .stores
            .get(secret.store())
            .and_then(|store| store.get(secret.name()))
        {
            return Ok(Secret(value.clone()));
        }
        if let Some(template) = self.resolvers.get(secret.store()) {
            return run(template, secret);
        }
        bail!("{secret} is not defined in {}", self.path.display())
    }
}

type Parsed = (
    BTreeMap<String, CommandTemplate>,
    BTreeMap<String, BTreeMap<String, String>>,
);

fn parse(text: &str) -> Result<Parsed, String> {
    let mut table: toml::Table = toml::from_str(text).map_err(|e| e.message().to_owned())?;
    let resolvers = match table.remove("resolvers") {
        Some(value) => value
            .try_into()
            .map_err(|e: toml::de::Error| e.message().to_owned())?,
        None => BTreeMap::new(),
    };
    let stores = toml::Value::Table(table)
        .try_into()
        .map_err(|_: toml::de::Error| "store values must be strings".to_owned())?;
    Ok((resolvers, stores))
}

fn run(template: &CommandTemplate, secret: &SecretRef) -> Result<Secret> {
    let command = template.0.replace("{name}", secret.name());
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .output()
        .with_context(|| format!("cannot run the resolver for {secret}"))?;
    if !output.status.success() {
        bail!(
            "resolver for {secret} failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let mut value = String::from_utf8(output.stdout)
        .map_err(|_| anyhow::anyhow!("resolver for {secret} produced non-UTF-8 output"))?;
    value.truncate(value.trim_end_matches(['\r', '\n']).len());
    if value.is_empty() {
        bail!("resolver for {secret} produced no value");
    }
    Ok(Secret(value))
}

#[cfg(unix)]
fn require_private(path: &Path, meta: &std::fs::Metadata) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let mode = meta.mode();
    if mode & 0o077 != 0 {
        bail!(
            "{} is readable by other users (mode {:o}); chmod 600 it",
            path.display(),
            mode & 0o777
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn require_private(_path: &Path, _meta: &std::fs::Metadata) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn temp_file(name: &str, contents: &str, mode: u32) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("reef-secrets-{}-{name}.toml", std::process::id()));
        std::fs::write(&path, contents).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    fn reference(text: &str) -> SecretRef {
        text.parse().unwrap()
    }

    #[test]
    fn resolves_and_redacts() {
        let path = temp_file("resolve", "[platform]\nanthropic = \"sk-test\"\n", 0o600);
        let secrets = Secrets::load(&path).unwrap();
        let value = secrets
            .resolve(&reference("reef://platform/anthropic"))
            .unwrap();
        assert_eq!(value.expose(), "sk-test");
        assert_eq!(format!("{value:?}"), "Secret(redacted)");

        assert!(
            secrets
                .resolve(&reference("reef://platform/other"))
                .is_err()
        );
    }

    #[test]
    fn resolver_runs_and_trims_the_newline() {
        let path = temp_file(
            "resolver-run",
            "[resolvers]\nfake = \"echo {name}-value\"\n",
            0o600,
        );
        let secrets = Secrets::load(&path).unwrap();
        let value = secrets
            .resolve(&reference("reef://fake/my-test-key"))
            .unwrap();
        assert_eq!(value.expose(), "my-test-key-value");
    }

    #[test]
    fn inline_value_wins_over_a_resolver() {
        let path = temp_file(
            "precedence",
            "[resolvers]\nvault = \"echo from-resolver # {name}\"\n[vault]\npinned = \"from-file\"\n",
            0o600,
        );
        let secrets = Secrets::load(&path).unwrap();
        assert_eq!(
            secrets
                .resolve(&reference("reef://vault/pinned"))
                .unwrap()
                .expose(),
            "from-file"
        );
        assert_eq!(
            secrets
                .resolve(&reference("reef://vault/other"))
                .unwrap()
                .expose(),
            "from-resolver"
        );
    }

    #[test]
    fn resolver_failure_and_empty_output_are_named_errors() {
        let path = temp_file(
            "resolver-err",
            "[resolvers]\nbad = \"false # {name}\"\nempty = \"true # {name}\"\n",
            0o600,
        );
        let secrets = Secrets::load(&path).unwrap();
        let failed = secrets.resolve(&reference("reef://bad/key")).unwrap_err();
        assert!(failed.to_string().contains("reef://bad/key"), "{failed}");
        let empty = secrets.resolve(&reference("reef://empty/key")).unwrap_err();
        assert!(empty.to_string().contains("no value"), "{empty}");
    }

    #[test]
    fn resolver_without_name_placeholder_is_rejected_at_load() {
        let path = temp_file(
            "resolver-shape",
            "[resolvers]\nbad = \"echo static\"\n",
            0o600,
        );
        let err = Secrets::load(&path)
            .err()
            .expect("static command must not load");
        assert!(err.to_string().contains("{name}"), "{err}");
    }

    #[test]
    fn refuses_world_readable_files() {
        let path = temp_file("world", "[platform]\nx = \"y\"\n", 0o644);
        assert!(Secrets::load(&path).is_err());
    }

    #[test]
    fn parse_errors_never_echo_the_line() {
        let path = temp_file(
            "parse-err",
            "[platform]\nkey = sk-live-SECRETVALUE\n",
            0o600,
        );
        let err = Secrets::load(&path)
            .err()
            .expect("bare value must not parse");
        assert!(!err.to_string().contains("SECRETVALUE"), "leaked: {err}");

        let path = temp_file("type-err", "top-level = \"sk-live-SECRETVALUE\"\n", 0o600);
        let err = Secrets::load(&path)
            .err()
            .expect("a bare top-level value must not parse");
        assert!(!err.to_string().contains("SECRETVALUE"), "leaked: {err}");
    }

    #[test]
    fn missing_file_is_empty() {
        let secrets = Secrets::load(Path::new("/nonexistent/reef-secrets.toml")).unwrap();
        assert!(secrets.resolve(&reference("reef://a/b")).is_err());
    }
}
