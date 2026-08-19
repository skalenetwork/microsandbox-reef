use crate::secrets::Secrets;
use crate::store::Store;
use crate::vmm::{SecretEnv, VmConfig, Vmm, VolumeMount};
use anyhow::{Context, Result};
use reef_core::{Action, Agent, AgentName, Desired, Facts, Lifecycle, Role, plan};

pub fn sandbox_name(agent: &AgentName) -> String {
    format!("reef-{agent}")
}

pub async fn reconcile<V: Vmm>(
    store: &Store,
    secrets: &Secrets,
    vmm: &V,
    name: &AgentName,
) -> Result<Agent> {
    let mut agent = store
        .get_agent(name)?
        .with_context(|| format!("no such agent: {name}"))?;
    let sandbox = sandbox_name(name);
    let vm = vmm.status(&sandbox).await?;
    let steps = plan(Facts {
        desired: agent.spec.desired,
        in_sync: agent.vm_current(),
        vm,
    });

    let result = run(store, secrets, vmm, &mut agent, &sandbox, steps).await;
    agent.status.lifecycle = match result {
        Ok(()) => {
            agent.status.applied_generation = agent.generation;
            match agent.spec.desired {
                Desired::Running => Lifecycle::Running,
                Desired::Stopped => Lifecycle::Stopped,
            }
        }
        Err(ref e) => {
            let reason = format!("{e:#}");
            store.record(name, "failed", &reason)?;
            Lifecycle::Failed { reason }
        }
    };
    store.set_status(name, &agent.status)?;
    result.map(|()| agent)
}

async fn run<V: Vmm>(
    store: &Store,
    secrets: &Secrets,
    vmm: &V,
    agent: &mut Agent,
    sandbox: &str,
    steps: &[Action],
) -> Result<()> {
    for step in steps {
        match step {
            Action::Create => {
                let role = store.role_version(&agent.spec.role_digest)?;
                let config = vm_config(store, secrets, agent, &role, sandbox)?;
                vmm.create(config).await?;
                agent.status.applied_digest = Some(agent.spec.role_digest.clone());
            }
            Action::Start => vmm.start(sandbox).await?,
            Action::Stop => vmm.stop(sandbox).await?,
            Action::Remove => vmm.remove(sandbox).await?,
        }
        store.record(&agent.name, step.label(), sandbox)?;
    }
    Ok(())
}

fn vm_config<'a>(
    store: &Store,
    secrets: &Secrets,
    agent: &Agent,
    role: &'a Role,
    sandbox: &str,
) -> Result<VmConfig<'a>> {
    let volume = agent
        .spec
        .workspace
        .as_ref()
        .map(|workspace| {
            store.ensure_workspace(workspace).map(|volume| VolumeMount {
                volume,
                dest: "/workspace".to_owned(),
            })
        })
        .transpose()?;
    let secret_envs = role
        .secrets
        .iter()
        .map(|(key, binding)| {
            secrets.resolve(&binding.secret).map(|value| SecretEnv {
                key,
                value,
                host: &binding.host,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(VmConfig {
        name: sandbox.to_owned(),
        role,
        secrets: secret_envs,
        volume,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;
    use reef_core::{AgentSpec, AgentStatus, Desired, Digest, VmStatus, parse_role};
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeVmm {
        vms: Mutex<HashMap<String, VmStatus>>,
        fail_create: bool,
    }

    impl Vmm for FakeVmm {
        async fn status(&self, name: &str) -> Result<Option<VmStatus>> {
            Ok(self.vms.lock().unwrap().get(name).copied())
        }

        async fn create(&self, config: VmConfig<'_>) -> Result<()> {
            if self.fail_create {
                bail!("image pull failed");
            }
            self.vms
                .lock()
                .unwrap()
                .insert(config.name, VmStatus::Running);
            Ok(())
        }

        async fn start(&self, name: &str) -> Result<()> {
            self.vms
                .lock()
                .unwrap()
                .insert(name.to_owned(), VmStatus::Running);
            Ok(())
        }

        async fn stop(&self, name: &str) -> Result<()> {
            self.vms
                .lock()
                .unwrap()
                .insert(name.to_owned(), VmStatus::Stopped);
            Ok(())
        }

        async fn remove(&self, name: &str) -> Result<()> {
            self.vms.lock().unwrap().remove(name);
            Ok(())
        }
    }

    const ROLE: &str = r#"
version = 1
name = "echo"
image = "alpine"
resources = { vcpus = 1, memory-mib = 256 }
network = { egress = ["example.com"] }
"#;

    fn setup() -> (Store, Secrets, Digest, AgentName) {
        let store = Store::open_temp();
        let secrets =
            Secrets::load(std::path::Path::new("/nonexistent/reef-secrets.toml")).unwrap();
        let role = parse_role(ROLE).unwrap();
        let digest: Digest = "b".repeat(64).parse().unwrap();
        let json = serde_json::to_string(&role).unwrap();
        store.import_role(&role, &digest, &json).unwrap();

        let name: AgentName = "worker-1".parse().unwrap();
        store
            .insert_agent(&Agent {
                name: name.clone(),
                generation: 1,
                spec: AgentSpec {
                    owner: "test".to_owned(),
                    role: role.name,
                    role_digest: digest.clone(),
                    workspace: None,
                    desired: Desired::Running,
                },
                status: AgentStatus {
                    lifecycle: Lifecycle::Pending,
                    applied_generation: 0,
                    applied_digest: None,
                },
            })
            .unwrap();
        (store, secrets, digest, name)
    }

    #[tokio::test]
    async fn creates_then_settles() {
        let (store, secrets, digest, name) = setup();
        let vmm = FakeVmm::default();

        let agent = reconcile(&store, &secrets, &vmm, &name).await.unwrap();
        assert!(agent.vm_current() && agent.reconciled());
        assert_eq!(agent.status.applied_digest, Some(digest));
        assert!(matches!(agent.status.lifecycle, Lifecycle::Running));

        let again = reconcile(&store, &secrets, &vmm, &name).await.unwrap();
        assert_eq!(again, agent);
        let history = store.history(&name).unwrap();
        assert_eq!(history.len(), 1, "second pass must be a no-op: {history:?}");
    }

    #[tokio::test]
    async fn stop_start_cycle_never_recreates() {
        let (store, secrets, _digest, name) = setup();
        let vmm = FakeVmm::default();
        reconcile(&store, &secrets, &vmm, &name).await.unwrap();

        store.set_desired(&name, Desired::Stopped, 1).unwrap();
        let agent = reconcile(&store, &secrets, &vmm, &name).await.unwrap();
        assert_eq!(agent.status.lifecycle, Lifecycle::Stopped);
        assert!(agent.reconciled());

        store.set_desired(&name, Desired::Running, 2).unwrap();
        let agent = reconcile(&store, &secrets, &vmm, &name).await.unwrap();
        assert!(agent.vm_current() && agent.reconciled());
        assert_eq!(agent.status.applied_generation, 3);
        let kinds: Vec<String> = store
            .history(&name)
            .unwrap()
            .into_iter()
            .map(|(_, kind, _)| kind)
            .collect();
        assert_eq!(kinds, ["create", "stop", "start"]);
    }

    #[tokio::test]
    async fn role_version_change_recreates() {
        let (store, secrets, _digest, name) = setup();
        let vmm = FakeVmm::default();
        reconcile(&store, &secrets, &vmm, &name).await.unwrap();

        let role = parse_role(&ROLE.replace("memory-mib = 256", "memory-mib = 320")).unwrap();
        let next: Digest = "c".repeat(64).parse().unwrap();
        let json = serde_json::to_string(&role).unwrap();
        store.import_role(&role, &next, &json).unwrap();
        store.set_role_digest(&name, &next, 1).unwrap();

        let agent = reconcile(&store, &secrets, &vmm, &name).await.unwrap();
        assert_eq!(agent.status.applied_digest, Some(next));
        assert!(agent.vm_current() && agent.reconciled());
        let kinds: Vec<String> = store
            .history(&name)
            .unwrap()
            .into_iter()
            .map(|(_, kind, _)| kind)
            .collect();
        assert_eq!(kinds, ["create", "remove", "create"]);
    }

    #[tokio::test]
    async fn failure_is_recorded_and_recoverable() {
        let (store, secrets, _digest, name) = setup();
        let failing = FakeVmm {
            fail_create: true,
            ..FakeVmm::default()
        };
        assert!(reconcile(&store, &secrets, &failing, &name).await.is_err());
        let agent = store.get_agent(&name).unwrap().unwrap();
        match &agent.status.lifecycle {
            Lifecycle::Failed { reason } => assert!(reason.contains("image pull failed")),
            other => panic!("expected failed, got {other:?}"),
        }

        let vmm = FakeVmm::default();
        let agent = reconcile(&store, &secrets, &vmm, &name).await.unwrap();
        assert!(matches!(agent.status.lifecycle, Lifecycle::Running));
    }
}
