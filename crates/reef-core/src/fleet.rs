use crate::name::{AgentName, EnvKey, RoleName, WorkspaceName};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fleet {
    pub version: u32,
    #[serde(default)]
    pub agents: BTreeMap<AgentName, FleetAgent>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetAgent {
    pub role: RoleName,
    #[serde(default)]
    pub workspace: Option<WorkspaceName>,
    #[serde(default)]
    pub env: BTreeMap<EnvKey, String>,
}

#[derive(Deserialize)]
struct VersionPeek {
    version: u32,
}

pub fn parse_fleet(text: &str) -> Result<Fleet, String> {
    let peek: VersionPeek = toml::from_str(text).map_err(|e| e.to_string())?;
    if peek.version != 1 {
        return Err(format!(
            "unsupported fleet version {} (this reef reads version 1)",
            peek.version
        ));
    }
    let fleet: Fleet = toml::from_str(text).map_err(|e| e.to_string())?;
    for (name, agent) in &fleet.agents {
        for (key, value) in &agent.env {
            if key.as_str().starts_with("MSB_") {
                return Err(format!(
                    "agents.{name}.env.{key}: the MSB_ prefix is reserved by the runtime"
                ));
            }
            if value.contains('\0') {
                return Err(format!(
                    "agents.{name}.env.{key}: NUL bytes are not allowed"
                ));
            }
        }
    }
    Ok(fleet)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_gates_version() {
        let fleet = parse_fleet(
            r#"
version = 1

[agents.ana-hermes]
role = "hermes"
env = { FOO = "bar" }

[agents.bob-hermes]
role = "hermes"
workspace = "bob-data"
"#,
        )
        .unwrap();
        assert_eq!(fleet.agents.len(), 2);
        let ana = &fleet.agents[&"ana-hermes".parse().unwrap()];
        assert_eq!(ana.env.len(), 1);

        assert!(parse_fleet("version = 1\n").unwrap().agents.is_empty());
        assert!(
            parse_fleet("version = 2\n")
                .unwrap_err()
                .contains("version 2")
        );
        let nul = "version = 1\n[agents.a]\nrole = \"r\"\nenv = { K = \"a\\u0000b\" }\n";
        assert!(parse_fleet(nul).unwrap_err().contains("NUL"));
        let msb = "version = 1\n[agents.a]\nrole = \"r\"\nenv = { MSB_X = \"y\" }\n";
        assert!(parse_fleet(msb).unwrap_err().contains("MSB_"));
    }
}
