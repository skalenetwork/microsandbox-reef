use crate::secrets::Secrets;
use crate::store::Store;
use crate::vmm::{SecretEnv, VmConfig, Vmm, VolumeMount};
use anyhow::{Context, Result, bail};
use reef_core::{
    Action, Agent, AgentName, Desired, Facts, Lifecycle, PortName, Role, allocate_ports, plan,
};
use std::collections::BTreeMap;

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
    let role = match steps.contains(&Action::Create) {
        true => {
            let role = store.role_version(&agent.spec.role_digest)?;
            for key in agent.spec.env.keys() {
                if role.secrets.contains_key(key) {
                    bail!("agent env {key} collides with a role secret");
                }
            }
            Some(role)
        }
        false => None,
    };
    for step in steps {
        match step {
            Action::Create => {
                let role = role.as_ref().expect("plan pairs Create with a role");
                let ports = allocate_ports(
                    role.expose.keys(),
                    &store.ports(&agent.name)?,
                    &store.used_ports()?,
                )
                .map_err(anyhow::Error::msg)?;
                store.set_ports(&agent.name, &ports)?;
                let config = vm_config(store, secrets, agent, role, sandbox, &ports)?;
                vmm.create(config).await?;
                agent.status.applied_digest = Some(agent.spec.role_digest.clone());
                agent.status.applied_env = agent.spec.env.clone();
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
    agent: &'a Agent,
    role: &'a Role,
    sandbox: &str,
    ports: &BTreeMap<PortName, u16>,
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
        env: role.env.iter().chain(&agent.spec.env).collect(),
        ports: role
            .expose
            .iter()
            .map(|(name, guest)| (ports[name], *guest))
            .collect(),
        secrets: secret_envs,
        volume,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;
    use reef_core::{AgentSpec, Desired, Digest, EnvKey, VmStatus, parse_role};
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeVmm {
        vms: Mutex<HashMap<String, VmStatus>>,
        seen_env: Mutex<Vec<(String, String)>>,
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
            *self.seen_env.lock().unwrap() = config
                .env
                .iter()
                .map(|(key, value)| (key.to_string(), (*value).clone()))
                .collect();
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
        let digest = import(&store, ROLE, "b");
        let name: AgentName = "worker-1".parse().unwrap();
        insert(&store, &name, &digest, BTreeMap::new());
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
        let kinds: Vec<String> = store
            .events(Some(&name), None)
            .unwrap()
            .into_iter()
            .map(|event| event.kind)
            .collect();
        assert_eq!(kinds.len(), 1, "second pass must be a no-op: {kinds:?}");
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
            .events(Some(&name), None)
            .unwrap()
            .into_iter()
            .map(|event| event.kind)
            .collect();
        assert_eq!(kinds, ["create", "stop", "start"]);
    }

    #[tokio::test]
    async fn role_version_change_recreates() {
        let (store, secrets, _digest, name) = setup();
        let vmm = FakeVmm::default();
        reconcile(&store, &secrets, &vmm, &name).await.unwrap();

        let next = import(
            &store,
            &ROLE.replace("memory-mib = 256", "memory-mib = 320"),
            "c",
        );
        store.set_role_digest(&name, &next, 1).unwrap();

        let agent = reconcile(&store, &secrets, &vmm, &name).await.unwrap();
        assert_eq!(agent.status.applied_digest, Some(next));
        assert!(agent.vm_current() && agent.reconciled());
        let kinds: Vec<String> = store
            .events(Some(&name), None)
            .unwrap()
            .into_iter()
            .map(|event| event.kind)
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

    fn import(store: &Store, text: &str, digest: &str) -> Digest {
        let role = parse_role(text).unwrap();
        let digest: Digest = digest.repeat(64).parse().unwrap();
        store
            .import_role(&role, &digest, &serde_json::to_string(&role).unwrap())
            .unwrap();
        digest
    }

    fn insert(store: &Store, name: &AgentName, digest: &Digest, env: BTreeMap<EnvKey, String>) {
        store
            .insert_agent(&Agent::new(
                name.clone(),
                false,
                AgentSpec {
                    owner: "test".to_owned(),
                    role: "echo".parse().unwrap(),
                    role_digest: digest.clone(),
                    workspace: None,
                    desired: Desired::Running,
                    env,
                },
            ))
            .unwrap();
    }

    #[tokio::test]
    async fn agent_env_overrides_role_env() {
        let (store, secrets, _digest, _name) = setup();
        let vmm = FakeVmm::default();
        let layered = import(
            &store,
            &ROLE.replace(
                "network =",
                "env = { FOO = \"role\", KEEP = \"role\" }\nnetwork =",
            ),
            "e",
        );
        let name: AgentName = "worker-2".parse().unwrap();
        insert(
            &store,
            &name,
            &layered,
            BTreeMap::from([
                ("FOO".parse().unwrap(), "agent".to_owned()),
                ("EXTRA".parse().unwrap(), "new".to_owned()),
            ]),
        );
        reconcile(&store, &secrets, &vmm, &name).await.unwrap();
        let env = vmm.seen_env.lock().unwrap().clone();
        assert_eq!(
            env,
            [
                ("EXTRA".to_owned(), "new".to_owned()),
                ("FOO".to_owned(), "agent".to_owned()),
                ("KEEP".to_owned(), "role".to_owned()),
            ]
        );
    }

    #[tokio::test]
    async fn agent_env_may_not_shadow_role_secrets() {
        let (store, secrets, _digest, _name) = setup();
        let vmm = FakeVmm::default();
        let secretful = import(
            &store,
            &ROLE.replace(
                "network =",
                "secrets = { FOO = { ref = \"reef://demo/fake\", host = \"example.com\" } }\nnetwork =",
            ),
            "f",
        );
        let name: AgentName = "worker-3".parse().unwrap();
        insert(
            &store,
            &name,
            &secretful,
            BTreeMap::from([("FOO".parse().unwrap(), "shadow".to_owned())]),
        );
        let err = reconcile(&store, &secrets, &vmm, &name).await.unwrap_err();
        assert!(err.to_string().contains("collides"), "{err}");
    }

    #[tokio::test]
    async fn env_change_recreates() {
        let (store, secrets, digest, name) = setup();
        let vmm = FakeVmm::default();
        let agent = reconcile(&store, &secrets, &vmm, &name).await.unwrap();

        store
            .set_fleet_spec(
                &name,
                &"echo".parse().unwrap(),
                &digest,
                &BTreeMap::from([("FOO".parse().unwrap(), "v2".to_owned())]),
                agent.generation,
            )
            .unwrap();
        let agent = reconcile(&store, &secrets, &vmm, &name).await.unwrap();
        assert!(agent.vm_current() && agent.reconciled());
        assert_eq!(
            *vmm.seen_env.lock().unwrap(),
            [("FOO".to_owned(), "v2".to_owned())]
        );
        let kinds: Vec<String> = store
            .events(Some(&name), None)
            .unwrap()
            .into_iter()
            .map(|event| event.kind)
            .collect();
        assert_eq!(kinds, ["create", "remove", "create"]);
    }

    #[tokio::test]
    async fn port_allocations_survive_updates() {
        let (store, secrets, plain, name) = setup();
        let vmm = FakeVmm::default();
        let agent = reconcile(&store, &secrets, &vmm, &name).await.unwrap();
        assert!(store.ports(&name).unwrap().is_empty());

        let ui = ROLE.replace("network =", "expose = { ui = 9119 }\nnetwork =");
        let v2 = import(&store, &ui, "c");
        store.set_role_digest(&name, &v2, agent.generation).unwrap();
        let agent = reconcile(&store, &secrets, &vmm, &name).await.unwrap();
        assert_eq!(store.ports(&name).unwrap()[&"ui".parse().unwrap()], 19000);

        let both = ROLE.replace(
            "network =",
            "expose = { metrics = 9100, ui = 9119 }\nnetwork =",
        );
        let v3 = import(&store, &both, "d");
        store.set_role_digest(&name, &v3, agent.generation).unwrap();
        let agent = reconcile(&store, &secrets, &vmm, &name).await.unwrap();
        let ports = store.ports(&name).unwrap();
        assert_eq!(ports[&"ui".parse().unwrap()], 19000, "allocation is stable");
        assert_eq!(ports[&"metrics".parse().unwrap()], 19001);

        store
            .set_role_digest(&name, &plain, agent.generation)
            .unwrap();
        reconcile(&store, &secrets, &vmm, &name).await.unwrap();
        assert!(
            store.ports(&name).unwrap().is_empty(),
            "removed entries are released"
        );
    }
}
