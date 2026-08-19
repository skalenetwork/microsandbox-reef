use anyhow::{Context, Result, bail};
use reef_core::{
    Agent, AgentName, AgentSpec, AgentStatus, Desired, Digest, Lifecycle, Role, RoleName,
    WorkspaceName,
};
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::path::Path;

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
CREATE TABLE workspaces (
  name       TEXT PRIMARY KEY,
  volume     TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE TABLE agents (
  name               TEXT PRIMARY KEY,
  generation         INTEGER NOT NULL,
  owner              TEXT NOT NULL,
  role               TEXT NOT NULL,
  role_digest        TEXT NOT NULL REFERENCES role_versions(digest),
  workspace          TEXT REFERENCES workspaces(name),
  desired            TEXT NOT NULL,
  lifecycle          TEXT NOT NULL,
  last_error         TEXT,
  applied_generation INTEGER NOT NULL,
  applied_digest     TEXT,
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
        if version < 1 {
            db.execute_batch(SCHEMA)?;
            db.pragma_update(None, "user_version", 1)?;
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
        Ok(decode_role((digest.as_str().to_owned(), definition))?.1)
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

    pub fn ensure_workspace(&self, name: &WorkspaceName) -> Result<String> {
        self.db.execute(
            "INSERT OR IGNORE INTO workspaces (name, volume, created_at)
             VALUES (?1, 'reef-ws-' || ?1, unixepoch())",
            [name.as_str()],
        )?;
        Ok(self.db.query_row(
            "SELECT volume FROM workspaces WHERE name = ?1",
            [name.as_str()],
            |row| row.get(0),
        )?)
    }

    pub fn insert_agent(&self, agent: &Agent) -> Result<()> {
        let inserted = self.db.execute(
            "INSERT OR IGNORE INTO agents
               (name, generation, owner, role, role_digest, workspace, desired,
                lifecycle, last_error, applied_generation, applied_digest,
                created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, unixepoch(), unixepoch())",
            params![
                agent.name.as_str(),
                agent.generation,
                agent.spec.owner,
                agent.spec.role.as_str(),
                agent.spec.role_digest.as_str(),
                agent.spec.workspace.as_ref().map(|w| w.as_str()),
                agent.spec.desired.as_str(),
                agent.status.lifecycle.label(),
                error_of(&agent.status.lifecycle),
                agent.status.applied_generation,
                agent.status.applied_digest.as_ref().map(|d| d.as_str()),
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
                "SELECT name, generation, owner, role, role_digest, workspace, desired,
                        lifecycle, last_error, applied_generation, applied_digest
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
            "SELECT name, generation, owner, role, role_digest, workspace, desired,
                    lifecycle, last_error, applied_generation, applied_digest
             FROM agents ORDER BY name",
        )?;
        let rows = stmt.query_map([], agent_row)?;
        rows.map(|raw| decode_agent(raw?))
            .collect::<Result<Vec<_>>>()
    }

    pub fn set_desired(&self, name: &AgentName, desired: Desired, expected: u64) -> Result<()> {
        let updated = self.db.execute(
            "UPDATE agents
             SET desired = ?1, generation = generation + 1, updated_at = unixepoch()
             WHERE name = ?2 AND generation = ?3",
            params![desired.as_str(), name.as_str(), expected],
        )?;
        if updated == 0 {
            bail!("agent {name} changed underneath this command; re-run it");
        }
        Ok(())
    }

    pub fn set_role_digest(&self, name: &AgentName, digest: &Digest, expected: u64) -> Result<()> {
        let updated = self.db.execute(
            "UPDATE agents
             SET role_digest = ?1, generation = generation + 1, updated_at = unixepoch()
             WHERE name = ?2 AND generation = ?3",
            params![digest.as_str(), name.as_str(), expected],
        )?;
        if updated == 0 {
            bail!("agent {name} changed underneath this command; re-run it");
        }
        Ok(())
    }

    pub fn set_status(&self, name: &AgentName, status: &AgentStatus) -> Result<()> {
        self.db.execute(
            "UPDATE agents
             SET lifecycle = ?1, last_error = ?2,
                 applied_generation = ?3, applied_digest = ?4, updated_at = unixepoch()
             WHERE name = ?5",
            params![
                status.lifecycle.label(),
                error_of(&status.lifecycle),
                status.applied_generation,
                status.applied_digest.as_ref().map(|d| d.as_str()),
                name.as_str(),
            ],
        )?;
        Ok(())
    }

    pub fn delete_agent(&self, name: &AgentName) -> Result<()> {
        self.db
            .execute("DELETE FROM agents WHERE name = ?1", [name.as_str()])?;
        Ok(())
    }

    pub fn record(&self, agent: &AgentName, kind: &str, detail: &str) -> Result<()> {
        self.db.execute(
            "INSERT INTO events (agent, at, kind, detail) VALUES (?1, unixepoch(), ?2, ?3)",
            params![agent.as_str(), kind, detail],
        )?;
        Ok(())
    }

    pub fn history(&self, agent: &AgentName) -> Result<Vec<(i64, String, String)>> {
        let mut stmt = self
            .db
            .prepare("SELECT at, kind, detail FROM events WHERE agent = ?1 ORDER BY id")?;
        let rows = stmt.query_map([agent.as_str()], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

type RawAgent = (
    String,
    u64,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    u64,
    Option<String>,
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
    ))
}

fn decode_agent(raw: RawAgent) -> Result<Agent> {
    let (
        name,
        generation,
        owner,
        role,
        role_digest,
        workspace,
        desired,
        lifecycle,
        last_error,
        applied_generation,
        applied_digest,
    ) = raw;
    let lifecycle = match (lifecycle.as_str(), last_error) {
        ("pending", _) => Lifecycle::Pending,
        ("running", _) => Lifecycle::Running,
        ("stopped", _) => Lifecycle::Stopped,
        ("failed", Some(reason)) => Lifecycle::Failed { reason },
        (other, _) => bail!("corrupt lifecycle row for {name}: {other:?}"),
    };
    Ok(Agent {
        name: name.parse().map_err(anyhow::Error::msg)?,
        generation,
        spec: AgentSpec {
            owner,
            role: role.parse().map_err(anyhow::Error::msg)?,
            role_digest: role_digest.parse().map_err(anyhow::Error::msg)?,
            workspace: workspace
                .map(|w| w.parse().map_err(anyhow::Error::msg))
                .transpose()?,
            desired: desired.parse().map_err(anyhow::Error::msg)?,
        },
        status: AgentStatus {
            lifecycle,
            applied_generation,
            applied_digest: applied_digest
                .map(|d| d.parse().map_err(anyhow::Error::msg))
                .transpose()?,
        },
    })
}

fn decode_role((digest, definition): (String, String)) -> Result<(Digest, Role)> {
    let digest: Digest = digest.parse().map_err(anyhow::Error::msg)?;
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

    fn open_temp() -> Store {
        let path = std::env::temp_dir().join(format!(
            "reef-test-{}-{:?}.db",
            std::process::id(),
            std::time::Instant::now()
        ));
        Store::open(&path).unwrap()
    }

    fn digest() -> Digest {
        "a".repeat(64).parse().unwrap()
    }

    #[test]
    fn role_import_is_idempotent_and_reports_activation() {
        let store = open_temp();
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
        let store = open_temp();
        let role = parse_role(ROLE).unwrap();
        let json = serde_json::to_string(&role).unwrap();
        store.import_role(&role, &digest(), &json).unwrap();

        let agent = Agent {
            name: "worker-1".parse().unwrap(),
            generation: 1,
            spec: AgentSpec {
                owner: "dmytro".into(),
                role: role.name.clone(),
                role_digest: digest(),
                workspace: None,
                desired: Desired::Running,
            },
            status: AgentStatus {
                lifecycle: Lifecycle::Pending,
                applied_generation: 0,
                applied_digest: None,
            },
        };
        store.insert_agent(&agent).unwrap();
        assert!(store.insert_agent(&agent).is_err());

        let loaded = store.get_agent(&agent.name).unwrap().unwrap();
        assert_eq!(loaded, agent);
        assert!(!loaded.reconciled());

        store.set_desired(&agent.name, Desired::Stopped, 1).unwrap();
        assert!(store.set_desired(&agent.name, Desired::Stopped, 1).is_err());
        let loaded = store.get_agent(&agent.name).unwrap().unwrap();
        assert_eq!(loaded.generation, 2);
        assert_eq!(loaded.spec.desired, Desired::Stopped);

        let status = AgentStatus {
            lifecycle: Lifecycle::Failed {
                reason: "boom".into(),
            },
            applied_generation: 2,
            applied_digest: Some(digest()),
        };
        store.set_status(&agent.name, &status).unwrap();
        let loaded = store.get_agent(&agent.name).unwrap().unwrap();
        assert_eq!(loaded.status, status);

        store
            .record(&agent.name, "create", "sandbox reef-worker-1")
            .unwrap();
        assert_eq!(store.history(&agent.name).unwrap().len(), 1);

        store.delete_agent(&agent.name).unwrap();
        assert!(store.get_agent(&agent.name).unwrap().is_none());
        assert_eq!(store.history(&agent.name).unwrap().len(), 1);
    }
}
