use crate::reconcile::host_name;
use reef_core::{
    AgentName, Desired, Digest, Domain, EnvKey, ImageRef, PortName, Resources, Role, RoleName,
    SecretBinding, State, VmStatus, VolumeName,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize)]
pub struct RoleRow {
    pub name: RoleName,
    pub digest: Digest,
    pub image: ImageRef,
    pub agents: usize,
    pub stale: usize,
}

#[derive(Serialize, Deserialize)]
pub struct RoleDetail {
    pub digest: Digest,
    #[serde(flatten)]
    pub role: Role,
    pub agents: Vec<AgentName>,
    pub stale: Vec<AgentName>,
}

impl RoleDetail {
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let role = &self.role;
        let mut rows = vec![
            ("name", role.name.to_string()),
            ("digest", self.digest.short().to_owned()),
            ("image", role.image.to_string()),
        ];
        if let Some(init) = &role.init {
            rows.push(("init", init.join(" ")));
        }
        rows.push((
            "resources",
            capacity(
                role.resources.vcpus,
                role.resources.memory_mib,
                role.resources.disk_gib,
            ),
        ));
        rows.extend(role.volumes.iter().map(|(name, volume)| {
            (
                "volume",
                format!("{name} {} {} MiB", volume.dest, volume.size_mib),
            )
        }));
        rows.push(("egress", egress(&role.network.egress)));
        rows.extend(role.secrets.iter().map(|(key, binding)| {
            (
                "secret",
                format!("{key}={} host={}", binding.secret, binding.host),
            )
        }));
        rows.extend(
            role.expose
                .iter()
                .map(|(name, port)| ("expose", format!("{name}={port}"))),
        );
        rows.extend(role.files.keys().map(|path| ("file", path.to_string())));
        rows.extend(
            role.env
                .iter()
                .map(|(key, value)| ("env", format!("{key}={value}"))),
        );
        rows.extend(self.agents.iter().map(|name| ("agent", name.to_string())));
        rows.extend(
            self.stale
                .iter()
                .map(|name| ("agent", format!("{name} (stale)"))),
        );
        rows
    }
}

#[derive(Serialize, Deserialize)]
pub struct AgentRow {
    pub name: AgentName,
    pub role: RoleName,
    pub role_digest: Digest,
    pub role_current: bool,
    pub image: ImageRef,
    pub owner: String,
    pub desired: Desired,
    pub state: State,
    pub vm: Option<VmStatus>,
    pub synced: bool,
    pub ports: BTreeMap<PortName, u16>,
}

#[derive(Serialize, Deserialize)]
pub struct AgentResources {
    pub vcpus: u8,
    pub memory_mib: u32,
    pub disk_gib: Option<u32>,
    pub max_pids: Option<u32>,
}

impl From<Resources> for AgentResources {
    fn from(resources: Resources) -> Self {
        Self {
            vcpus: resources.vcpus,
            memory_mib: resources.memory_mib,
            disk_gib: resources.disk_gib,
            max_pids: resources.max_pids,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct AgentDetail {
    pub name: AgentName,
    pub role: RoleName,
    pub role_digest: Digest,
    pub role_current: bool,
    pub image: ImageRef,
    pub owner: String,
    pub fleet: bool,
    pub resources: AgentResources,
    pub egress: Vec<Domain>,
    pub secrets: BTreeMap<EnvKey, SecretBinding>,
    pub volumes: BTreeMap<VolumeName, String>,
    pub desired: Desired,
    pub state: State,
    pub reason: Option<String>,
    pub generation: u64,
    pub applied_generation: u64,
    pub applied_digest: Option<Digest>,
    pub vm: Option<VmStatus>,
    pub sandbox: String,
    pub ports: BTreeMap<PortName, u16>,
    pub env: BTreeMap<EnvKey, String>,
}

impl AgentDetail {
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let stale = if self.role_current { "" } else { " (stale)" };
        let state = match &self.reason {
            Some(reason) => format!("{}: {reason}", self.state.label()),
            None => self.state.label().to_owned(),
        };
        let mut rows = vec![
            ("name", self.name.to_string()),
            (
                "role",
                format!("{}@{}{stale}", self.role, self.role_digest.short()),
            ),
            ("image", self.image.to_string()),
            ("owner", self.owner.clone()),
            ("desired", self.desired.label().to_owned()),
            ("state", state),
            ("vm", self.vm.map_or("-", VmStatus::label).to_owned()),
            ("sandbox", self.sandbox.clone()),
            (
                "resources",
                capacity(
                    self.resources.vcpus,
                    self.resources.memory_mib,
                    self.resources.disk_gib,
                ),
            ),
        ];
        rows.extend(
            self.volumes
                .iter()
                .map(|(entry, name)| ("volume", format!("{entry} {name}"))),
        );
        rows.push(("egress", egress(&self.egress)));
        rows.extend(self.secrets.iter().map(|(key, binding)| {
            (
                "secret",
                format!("{key}={} host={}", binding.secret, binding.host),
            )
        }));
        if !self.ports.is_empty() {
            let host = host_name(&self.name);
            let ports: Vec<String> = self
                .ports
                .iter()
                .map(|(name, port)| format!("{name}=http://{host}:{port}"))
                .collect();
            rows.push(("ports", ports.join(" ")));
        }
        rows.extend(
            self.env
                .iter()
                .map(|(key, value)| ("env", format!("{key}={value}"))),
        );
        let synced = self.generation == self.applied_generation;
        rows.push(("synced", if synced { "yes" } else { "drift" }.to_owned()));
        rows
    }
}

fn capacity(vcpus: u8, memory_mib: u32, disk_gib: Option<u32>) -> String {
    let disk = disk_gib.map_or(String::new(), |gib| format!(", {gib} GiB disk"));
    format!("{vcpus} vcpu, {memory_mib} MiB{disk}")
}

fn egress(domains: &[Domain]) -> String {
    if domains.is_empty() {
        return "none".to_owned();
    }
    domains
        .iter()
        .map(Domain::as_str)
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Serialize)]
pub struct Event {
    pub id: i64,
    pub agent: AgentName,
    pub at: i64,
    pub kind: String,
    pub detail: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROLE: &str = r#"
version = 1
name  = "echo"
image = "alpine"

[resources]
vcpus = 1
memory-mib = 256
max-pids = 128

[network]
egress = ["example.com"]
"#;

    fn round_trips<T: Serialize + serde::de::DeserializeOwned>(value: &T, json: &str) {
        assert_eq!(serde_json::to_string(value).unwrap(), json);
        let parsed: T = serde_json::from_str(json).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), json);
    }

    #[test]
    fn json_rows_are_a_stable_contract() {
        let digest = "0".repeat(64);
        round_trips(
            &RoleRow {
                name: "echo".parse().unwrap(),
                digest: digest.parse().unwrap(),
                image: "alpine".parse().unwrap(),
                agents: 2,
                stale: 1,
            },
            &format!(
                r#"{{"name":"echo","digest":"{digest}","image":"alpine","agents":2,"stale":1}}"#
            ),
        );

        round_trips(
            &RoleDetail {
                digest: digest.parse().unwrap(),
                role: reef_core::parse_role(ROLE).unwrap(),
                agents: vec!["echo-1".parse().unwrap()],
                stale: vec!["echo-2".parse().unwrap()],
            },
            &format!(
                r#"{{"digest":"{digest}","version":1,"name":"echo","image":"alpine","resources":{{"vcpus":1,"memory-mib":256,"disk-gib":null,"max-pids":128}},"network":{{"egress":["example.com"]}},"secrets":{{}},"agents":["echo-1"],"stale":["echo-2"]}}"#
            ),
        );

        let agent = AgentRow {
            name: "echo-1".parse().unwrap(),
            role: "echo".parse().unwrap(),
            role_digest: digest.parse().unwrap(),
            role_current: false,
            image: "alpine".parse().unwrap(),
            owner: "dmytro".to_owned(),
            desired: Desired::Running,
            state: State::Running,
            vm: None,
            synced: true,
            ports: BTreeMap::from([("ui".parse().unwrap(), 19007)]),
        };
        round_trips(
            &agent,
            &format!(
                r#"{{"name":"echo-1","role":"echo","role_digest":"{digest}","role_current":false,"image":"alpine","owner":"dmytro","desired":"running","state":"running","vm":null,"synced":true,"ports":{{"ui":19007}}}}"#
            ),
        );

        let event = Event {
            id: 7,
            agent: "echo-1".parse().unwrap(),
            at: 1,
            kind: "created".to_owned(),
            detail: "dmytro".to_owned(),
        };
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"id":7,"agent":"echo-1","at":1,"kind":"created","detail":"dmytro"}"#
        );

        let detail = AgentDetail {
            name: "echo-1".parse().unwrap(),
            role: "echo".parse().unwrap(),
            role_digest: digest.parse().unwrap(),
            role_current: true,
            image: "alpine".parse().unwrap(),
            owner: "dmytro".to_owned(),
            fleet: false,
            resources: AgentResources {
                vcpus: 2,
                memory_mib: 1024,
                disk_gib: None,
                max_pids: None,
            },
            egress: vec!["example.com".parse().unwrap()],
            secrets: BTreeMap::new(),
            volumes: BTreeMap::from([("data".parse().unwrap(), "reef-vol-echo-1-data".to_owned())]),
            desired: Desired::Running,
            state: State::Failed,
            reason: Some("boom".to_owned()),
            generation: 2,
            applied_generation: 1,
            applied_digest: None,
            vm: Some(VmStatus::Stopped),
            sandbox: "reef-echo-1".to_owned(),
            ports: BTreeMap::new(),
            env: BTreeMap::from([("FOO".parse().unwrap(), "bar".to_owned())]),
        };
        round_trips(
            &detail,
            &format!(
                r#"{{"name":"echo-1","role":"echo","role_digest":"{digest}","role_current":true,"image":"alpine","owner":"dmytro","fleet":false,"resources":{{"vcpus":2,"memory_mib":1024,"disk_gib":null,"max_pids":null}},"egress":["example.com"],"secrets":{{}},"volumes":{{"data":"reef-vol-echo-1-data"}},"desired":"running","state":"failed","reason":"boom","generation":2,"applied_generation":1,"applied_digest":null,"vm":"stopped","sandbox":"reef-echo-1","ports":{{}},"env":{{"FOO":"bar"}}}}"#
            ),
        );
    }
}
