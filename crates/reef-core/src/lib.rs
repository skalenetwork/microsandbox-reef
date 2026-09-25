mod agent;
mod fleet;
mod name;
mod plan;
mod ports;
mod role;

pub use agent::{Agent, AgentSpec, AgentStatus, Lifecycle, State};
pub use fleet::{Fleet, FleetAgent, parse_fleet};
pub use name::{
    AgentName, Digest, Domain, EnvKey, GuestPath, Host, ImageRef, PortName, RoleName, SecretRef,
    VolumeName,
};
pub use plan::{Action, Drift, VmStatus, plan};
pub use ports::{HOST_PORTS, allocate_ports};
pub use role::{File, Network, Resources, Role, SecretBinding, Volume, parse_role};

fn parse_v1<T: serde::de::DeserializeOwned>(text: &str, what: &str) -> Result<T, String> {
    #[derive(serde::Deserialize)]
    struct Peek {
        version: u32,
    }
    let Peek { version } = toml::from_str(text).map_err(|e| e.to_string())?;
    if version != 1 {
        return Err(format!(
            "unsupported {what} version {version} (this reef reads version 1)"
        ));
    }
    toml::from_str(text).map_err(|e| e.to_string())
}
