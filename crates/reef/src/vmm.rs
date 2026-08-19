use crate::secrets::Secret;
use anyhow::Result;
use reef_core::VmStatus;

pub struct VmConfig {
    pub name: String,
    pub image: String,
    pub vcpus: u8,
    pub memory_mib: u32,
    pub disk_gib: Option<u32>,
    pub max_pids: Option<u32>,
    pub egress: Vec<String>,
    pub secrets: Vec<SecretEnv>,
    pub volume: Option<VolumeMount>,
}

pub struct SecretEnv {
    pub key: String,
    pub value: Secret,
    pub host: String,
}

pub struct VolumeMount {
    pub volume: String,
    pub dest: String,
}

pub trait Vmm {
    async fn status(&self, name: &str) -> Result<Option<VmStatus>>;
    async fn create(&self, config: VmConfig) -> Result<()>;
    async fn start(&self, name: &str) -> Result<()>;
    async fn stop(&self, name: &str) -> Result<()>;
    async fn remove(&self, name: &str) -> Result<()>;
    async fn exec(&self, name: &str, command: &[String]) -> Result<i32>;
}
