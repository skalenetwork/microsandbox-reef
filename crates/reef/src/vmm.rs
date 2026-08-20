use crate::secrets::Secret;
use anyhow::Result;
use reef_core::{EnvKey, Host, Role, VmStatus};

pub struct VmConfig<'a> {
    pub name: String,
    pub role: &'a Role,
    pub secrets: Vec<SecretEnv<'a>>,
    pub volume: Option<VolumeMount>,
}

pub struct SecretEnv<'a> {
    pub key: &'a EnvKey,
    pub value: Secret,
    pub host: &'a Host,
}

pub struct VolumeMount {
    pub volume: String,
    pub dest: String,
}

pub trait Vmm {
    async fn status(&self, name: &str) -> Result<Option<VmStatus>>;
    async fn create(&self, config: VmConfig<'_>) -> Result<()>;
    async fn start(&self, name: &str) -> Result<()>;
    async fn stop(&self, name: &str) -> Result<()>;
    async fn remove(&self, name: &str) -> Result<()>;
}
