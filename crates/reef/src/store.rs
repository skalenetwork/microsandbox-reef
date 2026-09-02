use crate::rows::Event;
use anyhow::{Context, Result, bail};
use reef_core::{
    Agent, AgentName, AgentSpec, AgentStatus, Desired, Digest, EnvKey, Lifecycle, PortName, Role,
    RoleName,
};
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const SCHEMA_VERSION: i64 = 6;

const SCHEMA: &str = "
CREATE TABLE role_versions (
  digest      TEXT PRIMARY KEY,
  role        TEXT NOT NULL,
  definition  TEXT NOT NULL,
  imported_at INTEGER NOT NULL
);
CREATE TABLE roles (
  name          TEXT PRIMARY KEY,
  active_digest TEXT NOT NULL REFERENCES role_versions(digest)
);
CREATE TABLE agents (
  name               TEXT PRIMARY KEY,
  generation         INTEGER NOT NULL,
  fleet              INTEGER NOT NULL DEFAULT 0,
  owner              TEXT NOT NULL,
  role               TEXT NOT NULL,
  role_digest        TEXT NOT NULL REFERENCES role_versions(digest),
  desired            TEXT NOT NULL,
  env                TEXT NOT NULL DEFAULT '{}',
  lifecycle          TEXT NOT NULL,
  last_error         TEXT,
  applied_generation INTEGER NOT NULL,
  applied_digest     TEXT,
  applied_env        TEXT NOT NULL DEFAULT '{}',
  created_at         INTEGER NOT NULL,
  updated_at         INTEGER NOT NULL
);
CREATE TABLE events (
  id     INTEGER PRIMARY KEY AUTOINCREMENT,
  agent  TEXT NOT NULL,
  at     INTEGER NOT NULL,
  kind   TEXT NOT NULL,
  detail TEXT NOT NULL
);
CREATE TABLE agent_ports (
  agent TEXT NOT NULL,
  name  TEXT NOT NULL,
  port  INTEGER NOT NULL UNIQUE,
  PRIMARY KEY (agent, name)
);
";

pub struct Store {
    db: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let db = Connection::open(path)
            .with_context(|| format!("cannot open state db at {}", path.display()))?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        db.pragma_update(None, "busy_timeout", 5000)?;
        let version: i64 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        match version {
            0 => {
                db.execute_batch(SCHEMA)?;
                db.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            }
            SCHEMA_VERSION => {}
            _ => bail!(
                "{} is from another reef version (pre-release schema); \
                 delete it and re-create your agents",
                path.display()
            ),
        }
        Ok(Self { db })
    }

    pub fn import_role(&self, role: &Role, digest: &Digest, definition: &str) -> Result<bool> {
        self.db.execute(
            "INSERT OR IGNORE INTO role_versions (digest, role, definition, imported_at)
             VALUES (?1, ?2, ?3, unixepoch())",
            params![digest.as_str(), role.name.as_str(), definition],
        )?;
        let activated = self.db.execute(
            "INSERT INTO roles (name, active_digest) VALUES (?1, ?2)
             ON CONFLICT(name) DO UPDATE SET active_digest = excluded.active_digest
             WHERE roles.active_digest <> excluded.active_digest",
            params![role.name.as_str(), digest.as_str()],
        )?;
        Ok(activated > 0)
    }

    pub fn active_role(&self, name: &RoleName) -> Result<Option<(Digest, Role)>> {
        self.db
            .query_row(
                "SELECT v.digest, v.definition FROM roles r
                 JOIN role_versions v ON v.digest = r.active_digest
                 WHERE r.name = ?1",
                [name.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .map(decode_role)
            .transpose()
    }

    pub fn role_version(&self, digest: &Digest) -> Result<Role> {
        let definition: String = self
            .db
            .query_row(
                "SELECT definition FROM role_versions WHERE digest = ?1",
                [digest.as_str()],
                |row| row.get(0),
            )
            .optional()?
            .with_context(|| format!("role version {digest} is not in the store"))?;
        serde_json::from_str(&definition)
            .with_context(|| format!("corrupt role definition {digest}"))
    }

    pub fn list_roles(&self) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.db.prepare(
            "SELECT r.name, r.active_digest, json_extract(v.definition, '$.image')
             FROM roles r JOIN role_versions v ON v.digest = r.active_digest
             ORDER BY r.name",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn insert_agent(&self, agent: &Agent) -> Result<()> {
        let env = serde_json::to_string(&agent.spec.env)?;
        let inserted = self.db.execute(
            "INSERT OR IGNORE INTO agents
               (name, generation, fleet, owner, role, role_digest, desired, env,
                lifecycle, last_error, applied_generation, applied_digest, applied_env,
                created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     unixepoch(), unixepoch())",
            params![
                agent.name.as_str(),
                agent.generation,
                agent.fleet,
                agent.spec.owner,
                agent.spec.role.as_str(),
                agent.spec.role_digest.as_str(),
                agent.spec.desired.label(),
                env,
                agent.status.lifecycle.label(),
                error_of(&agent.status.lifecycle),
                agent.status.applied_generation,
                agent.status.applied_digest.as_ref().map(|d| d.as_str()),
                serde_json::to_string(&agent.status.applied_env)?,
            ],
        )?;
        if inserted == 0 {
            bail!("agent {} already exists", agent.name);
        }
        Ok(())
    }

    pub fn get_agent(&self, name: &AgentName) -> Result<Option<Agent>> {
        self.db
            .query_row(
                "SELECT name, generation, fleet, owner, role, role_digest, desired,
                        env, lifecycle, last_error, applied_generation, applied_digest, applied_env
                 FROM agents WHERE name = ?1",
                [name.as_str()],
                agent_row,
            )
            .optional()?
            .map(decode_agent)
            .transpose()
    }

    pub fn list_agents(&self) -> Result<Vec<Agent>> {
        let mut stmt = self.db.prepare(
            "SELECT name, generation, fleet, owner, role, role_digest, desired,
                    env, lifecycle, last_error, applied_generation, applied_digest, applied_env
             FROM agents ORDER BY name",
        )?;
        let rows = stmt.query_map([], agent_row)?;
        rows.map(|raw| decode_agent(raw?))
            .collect::<Result<Vec<_>>>()
    }

    pub fn set_desired(&self, name: &AgentName, desired: Desired, expected: u64) -> Result<()> {
        self.cas_set(name, "desired", desired.label(), expected)
    }

    pub fn set_role_digest(&self, name: &AgentName, digest: &Digest, expected: u64) -> Result<()> {
        self.cas_set(name, "role_digest", digest.as_str(), expected)
    }

    pub fn set_fleet_spec(
        &self,
        name: &AgentName,
        role: &RoleName,
        digest: &Digest,
        env: &BTreeMap<EnvKey, String>,
        owner: &str,
        expected: u64,
    ) -> Result<()> {
        let updated = self.db.execute(
            "UPDATE agents
             SET role = ?1, role_digest = ?2, env = ?3, owner = ?4,
                 generation = generation + 1, updated_at = unixepoch()
             WHERE name = ?5 AND generation = ?6",
            params![
                role.as_str(),
                digest.as_str(),
                serde_json::to_string(env)?,
                owner,
                name.as_str(),
                expected,
            ],
        )?;
        if updated == 0 {
            bail!("agent {name} changed underneath this command; re-run it");
        }
        Ok(())
    }

    pub fn fleet_agents(&self) -> Result<Vec<AgentName>> {
        let mut stmt = self
            .db
            .prepare("SELECT name FROM agents WHERE fleet ORDER BY name")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|name| parsed(name?)).collect()
    }

    fn cas_set(&self, name: &AgentName, column: &str, value: &str, expected: u64) -> Result<()> {
        let updated = self.db.execute(
            &format!(
                "UPDATE agents
                 SET {column} = ?1, generation = generation + 1, updated_at = unixepoch()
                 WHERE name = ?2 AND generation = ?3"
            ),
            params![value, name.as_str(), expected],
        )?;
        if updated == 0 {
            bail!("agent {name} changed underneath this command; re-run it");
        }
        Ok(())
    }

    pub fn set_status(&self, name: &AgentName, status: &AgentStatus) -> Result<()> {
        self.db.execute(
            "UPDATE agents
             SET lifecycle = ?1, last_error = ?2, applied_generation = ?3,
                 applied_digest = ?4, applied_env = ?5, updated_at = unixepoch()
             WHERE name = ?6",
            params![
                status.lifecycle.label(),
                error_of(&status.lifecycle),
                status.applied_generation,
                status.applied_digest.as_ref().map(|d| d.as_str()),
                serde_json::to_string(&status.applied_env)?,
                name.as_str(),
            ],
        )?;
        Ok(())
    }

    pub fn delete_agent(&self, name: &AgentName) -> Result<()> {
        let tx = self.db.unchecked_transaction()?;
        tx.execute("DELETE FROM agents WHERE name = ?1", [name.as_str()])?;
        tx.execute("DELETE FROM agent_ports WHERE agent = ?1", [name.as_str()])?;
        tx.commit()?;
        Ok(())
    }

    pub fn delete_fleet_agent(&self, name: &AgentName) -> Result<bool> {
        let tx = self.db.unchecked_transaction()?;
        let deleted = tx.execute(
            "DELETE FROM agents WHERE name = ?1 AND fleet",
            [name.as_str()],
        )?;
        if deleted > 0 {
            tx.execute("DELETE FROM agent_ports WHERE agent = ?1", [name.as_str()])?;
        }
        tx.commit()?;
        Ok(deleted > 0)
    }

    pub fn ports(&self, agent: &AgentName) -> Result<BTreeMap<PortName, u16>> {
        let mut stmt = self
            .db
            .prepare("SELECT name, port FROM agent_ports WHERE agent = ?1")?;
        let rows = stmt.query_map([agent.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get(1)?))
        })?;
        rows.map(|row| {
            let (name, port) = row?;
            Ok((parsed(name)?, port))
        })
        .collect()
    }

    pub fn used_ports(&self) -> Result<BTreeSet<u16>> {
        let mut stmt = self.db.prepare("SELECT port FROM agent_ports")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_ports(&self, agent: &AgentName, ports: &BTreeMap<PortName, u16>) -> Result<()> {
        let tx = self.db.unchecked_transaction()?;
        tx.execute("DELETE FROM agent_ports WHERE agent = ?1", [agent.as_str()])?;
        for (name, port) in ports {
            tx.execute(
                "INSERT INTO agent_ports (agent, name, port) VALUES (?1, ?2, ?3)",
                params![agent.as_str(), name.as_str(), port],
            )
            .with_context(|| format!("host port {port} is already allocated; re-run"))?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn record(&self, agent: &AgentName, kind: &str, detail: &str) -> Result<()> {
        self.db.execute(
            "INSERT INTO events (agent, at, kind, detail) VALUES (?1, unixepoch(), ?2, ?3)",
            params![agent.as_str(), kind, detail],
        )?;
        Ok(())
    }

    pub fn events(&self, agent: Option<&AgentName>, after: Option<i64>) -> Result<Vec<Event>> {
        let mut stmt = self.db.prepare(
            "SELECT id, agent, at, kind, detail FROM events
             WHERE (?1 IS NULL OR agent = ?1) AND (?2 IS NULL OR id > ?2)
             ORDER BY id",
        )?;
        let rows = stmt.query_map(params![agent.map(AgentName::as_str), after], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?;
        rows.map(|row| {
            let (id, agent, at, kind, detail): (i64, String, i64, String, String) = row?;
            Ok(Event {
                id,
                agent: parsed(agent)?,
                at,
                kind,
                detail,
            })
        })
        .collect()
    }
}

type RawAgent = (
    String,
    u64,
    bool,
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    u64,
    Option<String>,
    String,
);

fn agent_row(row: &Row<'_>) -> rusqlite::Result<RawAgent> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
    ))
}

fn decode_agent(raw: RawAgent) -> Result<Agent> {
    let (
        name,
        generation,
        fleet,
        owner,
        role,
        role_digest,
        desired,
        env,
        lifecycle,
        last_error,
        applied_generation,
        applied_digest,
        applied_env,
    ) = raw;
    let lifecycle = match (lifecycle.as_str(), last_error) {
        ("pending", _) => Lifecycle::Pending,
        ("running", _) => Lifecycle::Running,
        ("stopped", _) => Lifecycle::Stopped,
        ("failed", Some(reason)) => Lifecycle::Failed { reason },
        (other, _) => bail!("corrupt lifecycle row for {name}: {other:?}"),
    };
    Ok(Agent {
        name: parsed(name)?,
        generation,
        fleet,
        spec: AgentSpec {
            owner,
            role: parsed(role)?,
            role_digest: parsed(role_digest)?,
            desired: parsed(desired)?,
            env: serde_json::from_str(&env)?,
        },
        status: AgentStatus {
            lifecycle,
            applied_generation,
            applied_digest: applied_digest.map(parsed).transpose()?,
            applied_env: serde_json::from_str(&applied_env)?,
        },
    })
}

fn parsed<T: std::str::FromStr<Err = String>>(text: String) -> Result<T> {
    text.parse().map_err(anyhow::Error::msg)
}

fn decode_role((digest, definition): (String, String)) -> Result<(Digest, Role)> {
    let digest: Digest = parsed(digest)?;
    let role: Role = serde_json::from_str(&definition)
        .with_context(|| format!("corrupt role definition {digest}"))?;
    Ok((digest, role))
}

fn error_of(lifecycle: &Lifecycle) -> Option<&str> {
    match lifecycle {
        Lifecycle::Failed { reason } => Some(reason),
        _ => None,
    }
}

#[cfg(test)]
impl Store {
    pub(crate) fn open_temp() -> Self {
        Self::open(&Self::temp_path()).unwrap()
    }

    pub(crate) fn temp_path() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "reef-test-{}-{}.db",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reef_core::parse_role;

    const ROLE: &str = r#"
version = 1
name = "echo"
image = "alpine"
resources = { vcpus = 1, memory-mib = 256 }
network = { egress = ["example.com"] }
"#;

    fn digest() -> Digest {
        "a".repeat(64).parse().unwrap()
    }

    #[test]
    fn other_schema_versions_are_refused() {
        let path = Store::temp_path();
        {
            let store = Store::open(&path).unwrap();
            store.db.pragma_update(None, "user_version", 3).unwrap();
        }
        let err = Store::open(&path).err().expect("stale db must be refused");
        assert!(err.to_string().contains("delete it"), "{err}");
    }

    #[test]
    fn ports_roundtrip_and_release() {
        let store = Store::open_temp();
        let name: AgentName = "worker-1".parse().unwrap();
        let ports = BTreeMap::from([("ui".parse().unwrap(), 19000_u16)]);
        store.set_ports(&name, &ports).unwrap();
        assert_eq!(store.ports(&name).unwrap(), ports);
        assert_eq!(store.used_ports().unwrap(), BTreeSet::from([19000]));

        store.delete_agent(&name).unwrap();
        assert!(store.ports(&name).unwrap().is_empty());
        assert!(store.used_ports().unwrap().is_empty());
    }

    #[test]
    fn role_import_is_idempotent_and_reports_activation() {
        let store = Store::open_temp();
        let role = parse_role(ROLE).unwrap();
        let json = serde_json::to_string(&role).unwrap();
        assert!(store.import_role(&role, &digest(), &json).unwrap());
        assert!(!store.import_role(&role, &digest(), &json).unwrap());
        let (active, loaded) = store.active_role(&role.name).unwrap().unwrap();
        assert_eq!(active, digest());
        assert_eq!(loaded, role);
    }

    #[test]
    fn agent_roundtrip_and_cas() {
        let store = Store::open_temp();
        let role = parse_role(ROLE).unwrap();
        let json = serde_json::to_string(&role).unwrap();
        store.import_role(&role, &digest(), &json).unwrap();

        let agent = Agent::new(
            "worker-1".parse().unwrap(),
            true,
            AgentSpec {
                owner: "dmytro".to_owned(),
                role: role.name.clone(),
                role_digest: digest(),
                desired: Desired::Running,
                env: BTreeMap::from([("FOO".parse().unwrap(), "bar".to_owned())]),
            },
        );
        store.insert_agent(&agent).unwrap();
        assert!(store.insert_agent(&agent).is_err());

        let loaded = store.get_agent(&agent.name).unwrap().unwrap();
        assert_eq!(loaded, agent);
        assert!(!loaded.reconciled());
        assert_eq!(
            store.fleet_agents().unwrap(),
            std::slice::from_ref(&agent.name)
        );

        store.set_desired(&agent.name, Desired::Stopped, 1).unwrap();
        assert!(store.set_desired(&agent.name, Desired::Stopped, 1).is_err());
        let loaded = store.get_agent(&agent.name).unwrap().unwrap();
        assert_eq!(loaded.generation, 2);
        assert_eq!(loaded.spec.desired, Desired::Stopped);

        store
            .set_fleet_spec(
                &agent.name,
                &role.name,
                &digest(),
                &BTreeMap::new(),
                "ana",
                2,
            )
            .unwrap();
        assert!(
            store
                .set_fleet_spec(
                    &agent.name,
                    &role.name,
                    &digest(),
                    &BTreeMap::new(),
                    "ana",
                    2
                )
                .is_err()
        );
        let loaded = store.get_agent(&agent.name).unwrap().unwrap();
        assert_eq!(loaded.generation, 3);
        assert_eq!(loaded.spec.owner, "ana");
        assert!(loaded.spec.env.is_empty());

        let status = AgentStatus {
            lifecycle: Lifecycle::Failed {
                reason: "boom".to_owned(),
            },
            applied_generation: 2,
            applied_digest: Some(digest()),
            applied_env: BTreeMap::from([("FOO".parse().unwrap(), "bar".to_owned())]),
        };
        store.set_status(&agent.name, &status).unwrap();
        let loaded = store.get_agent(&agent.name).unwrap().unwrap();
        assert_eq!(loaded.status, status);

        store
            .record(&agent.name, "create", "sandbox reef-worker-1")
            .unwrap();
        let events = store.events(Some(&agent.name), None).unwrap();
        assert_eq!(events.len(), 1);

        store.record(&agent.name, "remove", "").unwrap();
        let after = store.events(None, Some(events[0].id)).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].kind, "remove");

        store.delete_agent(&agent.name).unwrap();
        assert!(store.get_agent(&agent.name).unwrap().is_none());
        assert_eq!(store.events(None, None).unwrap().len(), 2);
    }
}
