use crate::secrets::Secret;
use anyhow::Result;
use reef_core::{EnvKey, Host, Role, VmStatus};
use std::collections::BTreeMap;

pub struct VmConfig<'a> {
    pub name: String,
    pub role: &'a Role,
    pub env: BTreeMap<String, String>,
    pub ports: Vec<(u16, u16)>,
    pub secrets: Vec<SecretEnv<'a>>,
    pub volumes: Vec<VolumeMount>,
}

pub struct SecretEnv<'a> {
    pub key: &'a EnvKey,
    pub value: Secret,
    pub host: &'a Host,
}

pub struct VolumeMount {
    pub name: String,
    pub dest: String,
    pub quota_mib: u32,
}

pub trait Vmm {
    async fn status(&self, name: &str) -> Result<Option<VmStatus>>;
    async fn create(&self, config: VmConfig<'_>) -> Result<()>;
    async fn modify(&self, name: &str, env: BTreeMap<&EnvKey, Option<&String>>) -> Result<()>;
    async fn start(&self, name: &str) -> Result<()>;
    async fn stop(&self, name: &str) -> Result<()>;
    async fn remove(&self, name: &str) -> Result<()>;
}
