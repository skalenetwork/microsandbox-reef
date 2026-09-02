use crate::reconcile::host_name;
use reef_core::{
    AgentName, Desired, Digest, Domain, EnvKey, ImageRef, PortName, Resources, RoleName,
    SecretBinding, State, VmStatus, VolumeName,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Serialize)]
pub struct RoleRow {
    pub name: String,
    pub digest: String,
    pub image: String,
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
        let disk = self
            .resources
            .disk_gib
            .map_or(String::new(), |gib| format!(", {gib} GiB disk"));
        let mut rows = vec![
            ("name", self.name.to_string()),
            (
                "role",
                format!("{}@{}{stale}", self.role, &self.role_digest.as_str()[..12]),
            ),
            ("image", self.image.to_string()),
            ("owner", self.owner.clone()),
            ("desired", self.desired.label().to_owned()),
            ("state", state),
            ("vm", self.vm.map_or("-", VmStatus::label).to_owned()),
            ("sandbox", self.sandbox.clone()),
            (
                "resources",
                format!(
                    "{} vcpu, {} MiB{disk}",
                    self.resources.vcpus, self.resources.memory_mib
                ),
            ),
        ];
        rows.extend(
            self.volumes
                .iter()
                .map(|(entry, name)| ("volume", format!("{entry} {name}"))),
        );
        let egress: Vec<&str> = self.egress.iter().map(Domain::as_str).collect();
        rows.push((
            "egress",
            if egress.is_empty() {
                "none".to_owned()
            } else {
                egress.join(" ")
            },
        ));
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

    fn round_trips<T: Serialize + serde::de::DeserializeOwned>(value: &T, json: &str) {
        assert_eq!(serde_json::to_string(value).unwrap(), json);
        let parsed: T = serde_json::from_str(json).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), json);
    }

    #[test]
    fn json_rows_are_a_stable_contract() {
        let role = RoleRow {
            name: "echo".to_owned(),
            digest: "abc".to_owned(),
            image: "alpine".to_owned(),
        };
        assert_eq!(
            serde_json::to_string(&role).unwrap(),
            r#"{"name":"echo","digest":"abc","image":"alpine"}"#
        );

        let digest = "0".repeat(64);
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
