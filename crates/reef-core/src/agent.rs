use crate::name::{AgentName, Digest, RoleName, WorkspaceName};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Desired {
    Running,
    Stopped,
}

impl Desired {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Stopped => "stopped",
        }
    }
}

impl std::str::FromStr for Desired {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "running" => Ok(Self::Running),
            "stopped" => Ok(Self::Stopped),
            other => Err(format!("invalid desired state: {other:?}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lifecycle {
    Pending,
    Running,
    Stopped,
    Failed { reason: String },
}

impl Lifecycle {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::Failed { .. } => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSpec {
    pub owner: String,
    pub role: RoleName,
    pub role_digest: Digest,
    pub workspace: Option<WorkspaceName>,
    pub desired: Desired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStatus {
    pub lifecycle: Lifecycle,
    pub applied_generation: u64,
    pub applied_digest: Option<Digest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    pub name: AgentName,
    pub generation: u64,
    pub spec: AgentSpec,
    pub status: AgentStatus,
}

impl Agent {
    pub fn vm_current(&self) -> bool {
        self.status.applied_digest.as_ref() == Some(&self.spec.role_digest)
    }

    pub fn reconciled(&self) -> bool {
        self.generation == self.status.applied_generation
    }

    pub fn settled(&self) -> bool {
        self.reconciled()
            && match self.spec.desired {
                Desired::Running => self.status.lifecycle == Lifecycle::Running,
                Desired::Stopped => self.status.lifecycle == Lifecycle::Stopped,
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(desired: Desired, lifecycle: Lifecycle, applied_generation: u64) -> Agent {
        Agent {
            name: "a".parse().unwrap(),
            generation: 2,
            spec: AgentSpec {
                owner: "o".to_owned(),
                role: "r".parse().unwrap(),
                role_digest: "0".repeat(64).parse().unwrap(),
                workspace: None,
                desired,
            },
            status: AgentStatus {
                lifecycle,
                applied_generation,
                applied_digest: None,
            },
        }
    }

    #[test]
    fn settled_needs_reconciled_and_matching_lifecycle() {
        assert!(agent(Desired::Running, Lifecycle::Running, 2).settled());
        assert!(agent(Desired::Stopped, Lifecycle::Stopped, 2).settled());
        assert!(!agent(Desired::Running, Lifecycle::Running, 1).settled());
        assert!(!agent(Desired::Running, Lifecycle::Stopped, 2).settled());
        assert!(!agent(Desired::Running, Lifecycle::Pending, 2).settled());
        let failed = Lifecycle::Failed {
            reason: "boom".to_owned(),
        };
        assert!(!agent(Desired::Running, failed, 2).settled());
    }
}
