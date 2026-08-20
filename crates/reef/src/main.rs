mod msb;
mod reconcile;
mod secrets;
mod store;
mod update;
mod vmm;

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use reef_core::{
    Agent, AgentName, AgentSpec, Desired, Digest, EnvKey, Lifecycle, PortName, Role, RoleName,
    VmStatus, WorkspaceName, parse_fleet, parse_role,
};
use secrets::Secrets;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use store::Store;
use vmm::Vmm;

#[derive(Parser)]
#[command(
    name = "reef",
    version,
    about = "Declared agents, disposable microVMs",
    help_template = "{name} {version}\n{about}\n\n{usage-heading} {usage}\n\n{all-args}"
)]
struct Cli {
    /// State directory (db + secrets.toml)
    #[arg(long, global = true, env = "REEF_STATE", value_name = "DIR")]
    state: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Manage role definitions
    Role {
        #[command(subcommand)]
        command: RoleCommand,
    },
    /// Manage agents
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Manage the declared agent fleet
    Fleet {
        #[command(subcommand)]
        command: FleetCommand,
    },
    /// Show the event log
    Events {
        /// Only this agent's events
        #[arg(long)]
        agent: Option<AgentName>,
        /// Only events after this id
        #[arg(long, value_name = "ID")]
        after: Option<i64>,
        /// Print JSON
        #[arg(long)]
        json: bool,
    },
    /// Check this host can run agents
    Doctor,
    /// Replace this binary with the latest release
    Update,
}

#[derive(Subcommand)]
enum FleetCommand {
    /// Converge agents toward fleet files: create and update
    Apply {
        files: Vec<PathBuf>,
        /// Also remove fleet agents these files no longer declare
        #[arg(long)]
        prune: bool,
    },
}

#[derive(Subcommand)]
enum RoleCommand {
    /// Validate and import role files, activating each new version
    Apply { files: Vec<PathBuf> },
    /// List roles and their active versions
    List {
        /// Print JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum AgentCommand {
    /// Create an agent from a role and reconcile it
    Create {
        #[arg(long)]
        role: RoleName,
        #[arg(long)]
        name: AgentName,
        #[arg(long)]
        workspace: Option<WorkspaceName>,
        /// KEY=VALUE for this agent, overriding the role's [env] (repeatable)
        #[arg(long, value_name = "KEY=VALUE")]
        env: Vec<EnvPair>,
    },
    /// List agents with their observed VM state
    List {
        /// Print JSON
        #[arg(long)]
        json: bool,
    },
    /// Show one agent in detail, including any failure reason
    Get {
        name: AgentName,
        /// Block until the agent settles; exit nonzero if it failed
        #[arg(long)]
        wait: bool,
        /// Print JSON
        #[arg(long)]
        json: bool,
    },
    /// Run a command inside an agent's VM
    Exec {
        name: AgentName,
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    /// Forward local TCP ports into an agent's VM until interrupted
    Forward {
        name: AgentName,
        /// GUEST or LOCAL:GUEST (LOCAL 0 picks a free port); omit to list what the VM is listening on
        ports: Vec<PortSpec>,
    },
    /// Re-pin to the role's active version and recreate the VM
    Update { name: AgentName },
    /// Set desired state to running and reconcile
    Start { name: AgentName },
    /// Set desired state to stopped and reconcile
    Stop { name: AgentName },
    /// Destroy the VM and the record; workspaces survive
    Rm { name: AgentName },
}

#[derive(Clone)]
struct EnvPair(EnvKey, String);

impl std::str::FromStr for EnvPair {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (key, value) = value
            .split_once('=')
            .ok_or_else(|| format!("expected KEY=VALUE, got {value:?}"))?;
        Ok(Self(key.parse()?, value.to_owned()))
    }
}

#[derive(Clone, Copy)]
struct PortSpec {
    local: u16,
    guest: u16,
}

impl std::str::FromStr for PortSpec {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (local, guest) = match value.split_once(':') {
            Some((local, guest)) => (local, guest),
            None => (value, value),
        };
        let port = |text: &str| {
            text.parse::<u16>()
                .map_err(|_| format!("invalid port: {text:?}"))
        };
        let spec = Self {
            local: port(local)?,
            guest: port(guest)?,
        };
        if spec.guest == 0 {
            return Err("guest port cannot be 0".to_owned());
        }
        Ok(spec)
    }
}

#[derive(Serialize)]
struct RoleRow {
    name: String,
    digest: String,
    image: String,
}

#[derive(Serialize)]
struct AgentRow {
    name: AgentName,
    role: RoleName,
    owner: String,
    desired: &'static str,
    state: &'static str,
    vm: Option<&'static str>,
    synced: bool,
    ports: BTreeMap<PortName, u16>,
}

#[derive(Serialize)]
struct AgentDetail {
    name: AgentName,
    role: RoleName,
    role_digest: Digest,
    owner: String,
    fleet: bool,
    workspace: Option<WorkspaceName>,
    desired: &'static str,
    state: &'static str,
    reason: Option<String>,
    generation: u64,
    applied_generation: u64,
    applied_digest: Option<Digest>,
    vm: Option<&'static str>,
    sandbox: String,
    ports: BTreeMap<PortName, u16>,
    env: BTreeMap<EnvKey, String>,
}

struct Ctx {
    store: Store,
    secrets: Secrets,
    vmm: msb::Msb,
}

impl Ctx {
    fn open(dir: &Path) -> Result<Self> {
        Ok(Self {
            store: Store::open(&dir.join("reef.db"))?,
            secrets: Secrets::load(&dir.join("secrets.toml"))?,
            vmm: msb::Msb::new(dir),
        })
    }
}

fn default_state_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("XDG_STATE_HOME") {
        return Ok(PathBuf::from(dir).join("reef"));
    }
    let home = std::env::var("HOME").context("HOME is not set; pass --state")?;
    Ok(PathBuf::from(home).join(".local/state/reef"))
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        Cli::command().print_help()?;
        return Ok(());
    };
    let dir = cli.state.map_or_else(default_state_dir, Ok)?;
    let notice = (!matches!(command, Command::Update))
        .then(|| update::Notice::start(&dir))
        .flatten();
    let result = match command {
        Command::Role { command } => role_command(Ctx::open(&dir)?, command),
        Command::Agent { command } => agent_command(Ctx::open(&dir)?, command).await,
        Command::Fleet { command } => fleet_command(Ctx::open(&dir)?, command).await,
        Command::Events { agent, after, json } => {
            events_command(Ctx::open(&dir)?, agent, after, json)
        }
        Command::Doctor => msb::doctor(),
        Command::Update => update::run().await,
    };
    if let Some(notice) = notice {
        notice.finish().await;
    }
    result
}

fn role_command(ctx: Ctx, command: RoleCommand) -> Result<()> {
    match command {
        RoleCommand::Apply { files } => {
            if files.is_empty() {
                bail!("no role files given");
            }
            let mut failed = false;
            for file in &files {
                let text = std::fs::read_to_string(file)
                    .with_context(|| format!("cannot read {}", file.display()))?;
                match parse_role(&text) {
                    Ok(role) => {
                        let (digest, definition) = digest_role(&role);
                        let activated = ctx.store.import_role(&role, &digest, &definition)?;
                        let state = if activated { "active" } else { "unchanged" };
                        println!(
                            "{} {}@{} ({state})",
                            file.display(),
                            role.name,
                            short(digest.as_str())
                        );
                    }
                    Err(e) => {
                        failed = true;
                        eprintln!("{}: {e}", file.display());
                    }
                }
            }
            if failed {
                bail!("some role files were rejected");
            }
            Ok(())
        }
        RoleCommand::List { json } => {
            let roles: Vec<RoleRow> = ctx
                .store
                .list_roles()?
                .into_iter()
                .map(|(name, digest, image)| RoleRow {
                    name,
                    digest,
                    image,
                })
                .collect();
            if json {
                println!("{}", serde_json::to_string_pretty(&roles)?);
            } else {
                for role in &roles {
                    println!("{:24} {} {}", role.name, short(&role.digest), role.image);
                }
            }
            Ok(())
        }
    }
}

async fn agent_command(ctx: Ctx, command: AgentCommand) -> Result<()> {
    match command {
        AgentCommand::Create {
            role,
            name,
            workspace,
            env,
        } => {
            let (digest, _) = ctx
                .store
                .active_role(&role)?
                .with_context(|| format!("no such role: {role} (run `reef role apply` first)"))?;
            if let Some(workspace) = &workspace {
                ctx.store.ensure_workspace(workspace)?;
            }
            let agent = Agent::new(
                name,
                false,
                AgentSpec {
                    owner: owner(),
                    role,
                    role_digest: digest,
                    workspace,
                    desired: Desired::Running,
                    env: env
                        .into_iter()
                        .map(|EnvPair(key, value)| (key, value))
                        .collect(),
                },
            );
            ctx.store.insert_agent(&agent)?;
            ctx.store
                .record(&agent.name, "created", &agent.spec.owner)?;
            let agent =
                reconcile::reconcile(&ctx.store, &ctx.secrets, &ctx.vmm, &agent.name).await?;
            println!("{} {}", agent.name, agent.status.lifecycle.label());
            Ok(())
        }
        AgentCommand::List { json } => {
            let mut agents = Vec::new();
            for agent in ctx.store.list_agents()? {
                let vm = ctx
                    .vmm
                    .status(&reconcile::sandbox_name(&agent.name))
                    .await?;
                let synced = agent.reconciled();
                let ports = ctx.store.ports(&agent.name)?;
                agents.push(AgentRow {
                    name: agent.name,
                    role: agent.spec.role,
                    owner: agent.spec.owner,
                    desired: agent.spec.desired.label(),
                    state: agent.status.lifecycle.label(),
                    vm: vm.map(VmStatus::label),
                    synced,
                    ports,
                });
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&agents)?);
            } else {
                println!(
                    "{name:24} {role:16} {owner:10} {desired:8} {state:8} {vm:8} SYNC",
                    name = "NAME",
                    role = "ROLE",
                    owner = "OWNER",
                    desired = "DESIRED",
                    state = "STATE",
                    vm = "VM",
                );
                for agent in &agents {
                    println!(
                        "{:24} {:16} {:10} {:8} {:8} {:8} {}",
                        agent.name.as_str(),
                        agent.role.as_str(),
                        agent.owner,
                        agent.desired,
                        agent.state,
                        agent.vm.unwrap_or("-"),
                        if agent.synced { "yes" } else { "drift" },
                    );
                }
            }
            Ok(())
        }
        AgentCommand::Get { name, wait, json } => {
            let mut agent = require_agent(&ctx, &name)?;
            if wait {
                while !agent.settled()
                    && !matches!(agent.status.lifecycle, Lifecycle::Failed { .. })
                {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    agent = require_agent(&ctx, &name)?;
                }
            }
            let sandbox = reconcile::sandbox_name(&agent.name);
            let vm = ctx.vmm.status(&sandbox).await?;
            let state = agent.status.lifecycle.label();
            let reason = match agent.status.lifecycle {
                Lifecycle::Failed { reason } => Some(reason),
                _ => None,
            };
            let ports = ctx.store.ports(&agent.name)?;
            let detail = AgentDetail {
                name: agent.name,
                role: agent.spec.role,
                role_digest: agent.spec.role_digest,
                owner: agent.spec.owner,
                fleet: agent.fleet,
                workspace: agent.spec.workspace,
                desired: agent.spec.desired.label(),
                state,
                reason,
                generation: agent.generation,
                applied_generation: agent.status.applied_generation,
                applied_digest: agent.status.applied_digest,
                vm: vm.map(VmStatus::label),
                sandbox,
                ports,
                env: agent.spec.env,
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&detail)?);
            } else {
                row("name", detail.name.as_str());
                row(
                    "role",
                    format_args!("{}@{}", detail.role, short(detail.role_digest.as_str())),
                );
                row("owner", &detail.owner);
                row(
                    "workspace",
                    detail.workspace.as_ref().map_or("-", |w| w.as_str()),
                );
                row("desired", detail.desired);
                match &detail.reason {
                    Some(reason) => row("state", format_args!("{}: {reason}", detail.state)),
                    None => row("state", detail.state),
                }
                row("vm", detail.vm.unwrap_or("-"));
                row("sandbox", &detail.sandbox);
                if !detail.ports.is_empty() {
                    let ports: Vec<String> = detail
                        .ports
                        .iter()
                        .map(|(name, port)| format!("{name}=127.0.0.1:{port}"))
                        .collect();
                    row("ports", ports.join(" "));
                }
                for (key, value) in &detail.env {
                    row("env", format_args!("{key}={value}"));
                }
                row(
                    "synced",
                    if detail.generation == detail.applied_generation {
                        "yes"
                    } else {
                        "drift"
                    },
                );
            }
            if wait && detail.reason.is_some() {
                std::process::exit(1);
            }
            Ok(())
        }
        AgentCommand::Exec { name, command } => {
            require_agent(&ctx, &name)?;
            let code = ctx
                .vmm
                .exec(&reconcile::sandbox_name(&name), &command)
                .await?;
            std::process::exit(code);
        }
        AgentCommand::Forward { name, ports } if ports.is_empty() => {
            require_agent(&ctx, &name)?;
            let listening = ctx.vmm.listening(&reconcile::sandbox_name(&name)).await?;
            if listening.is_empty() {
                println!("{name} is not listening on any port");
                return Ok(());
            }
            let ports: Vec<String> = listening.iter().map(u16::to_string).collect();
            let ports = ports.join(" ");
            println!("{name} is listening on {ports}");
            println!("reef agent forward {name} {ports}");
            Ok(())
        }
        AgentCommand::Forward { name, ports } => {
            require_agent(&ctx, &name)?;
            let ports: Vec<(u16, u16)> = ports.iter().map(|p| (p.local, p.guest)).collect();
            ctx.vmm
                .forward(&reconcile::sandbox_name(&name), &ports)
                .await
        }
        AgentCommand::Update { name } => {
            let agent = require_agent(&ctx, &name)?;
            let (active, _) = ctx
                .store
                .active_role(&agent.spec.role)?
                .with_context(|| format!("role {} no longer exists", agent.spec.role))?;
            if active == agent.spec.role_digest {
                println!(
                    "{name} is already on {}@{}",
                    agent.spec.role,
                    short(active.as_str())
                );
                return Ok(());
            }
            ctx.store
                .set_role_digest(&name, &active, agent.generation)?;
            ctx.store.record(&name, "updated", active.as_str())?;
            let agent = reconcile::reconcile(&ctx.store, &ctx.secrets, &ctx.vmm, &name).await?;
            println!(
                "{} {} on {}@{}",
                agent.name,
                agent.status.lifecycle.label(),
                agent.spec.role,
                short(active.as_str())
            );
            Ok(())
        }
        AgentCommand::Start { name } => set_desired(&ctx, &name, Desired::Running).await,
        AgentCommand::Stop { name } => set_desired(&ctx, &name, Desired::Stopped).await,
        AgentCommand::Rm { name } => {
            let agent = require_agent(&ctx, &name)?;
            ctx.vmm.remove(&reconcile::sandbox_name(&name)).await?;
            ctx.store.delete_agent(&name)?;
            ctx.store.record(&name, "deleted", &agent.spec.owner)?;
            match agent.spec.workspace {
                Some(workspace) => println!("{name} removed (workspace {workspace} kept)"),
                None => println!("{name} removed"),
            }
            Ok(())
        }
    }
}

fn events_command(ctx: Ctx, agent: Option<AgentName>, after: Option<i64>, json: bool) -> Result<()> {
    let events = ctx.store.events(agent.as_ref(), after)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&events)?);
    } else {
        for event in &events {
            println!(
                "{:6} {} {:24} {:8} {}",
                event.id, event.at, event.agent, event.kind, event.detail
            );
        }
    }
    Ok(())
}

async fn set_desired(ctx: &Ctx, name: &AgentName, desired: Desired) -> Result<()> {
    let agent = require_agent(ctx, name)?;
    if agent.spec.desired != desired {
        ctx.store.set_desired(name, desired, agent.generation)?;
    }
    let agent = reconcile::reconcile(&ctx.store, &ctx.secrets, &ctx.vmm, name).await?;
    println!("{} {}", agent.name, agent.status.lifecycle.label());
    Ok(())
}

fn require_agent(ctx: &Ctx, name: &AgentName) -> Result<Agent> {
    ctx.store
        .get_agent(name)?
        .with_context(|| format!("no such agent: {name}"))
}

fn digest_role(role: &Role) -> (Digest, String) {
    let definition = serde_json::to_string(role).expect("roles serialize");
    let hash = Sha256::digest(definition.as_bytes());
    let hex: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
    (hex.parse().expect("sha256 hex is a digest"), definition)
}

fn short(digest: &str) -> &str {
    &digest[..12]
}

fn row(label: &str, value: impl std::fmt::Display) {
    println!("{label:10} {value}");
}

fn owner() -> String {
    std::env::var("USER").unwrap_or_else(|_| "unknown".to_owned())
}

async fn fleet_command(ctx: Ctx, command: FleetCommand) -> Result<()> {
    let FleetCommand::Apply { files, prune } = command;
    if files.is_empty() {
        bail!("no fleet files given");
    }
    let mut desired = BTreeMap::new();
    for file in &files {
        let text = std::fs::read_to_string(file)
            .with_context(|| format!("cannot read {}", file.display()))?;
        let fleet = parse_fleet(&text).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
        for (name, entry) in fleet.agents {
            if desired.insert(name.clone(), entry).is_some() {
                bail!("{name} is declared in more than one fleet file");
            }
        }
    }
    let mut digests = BTreeMap::new();
    for (name, entry) in &desired {
        let (digest, _) = ctx
            .store
            .active_role(&entry.role)?
            .with_context(|| format!("{name}: no such role: {}", entry.role))?;
        digests.insert(name.clone(), digest);
    }
    let mut failed = false;
    for name in ctx.store.fleet_agents()? {
        if desired.contains_key(&name) {
            continue;
        }
        if !prune {
            eprintln!("{name}: fleet-managed but not declared here; --prune removes it");
            continue;
        }
        match ctx.vmm.remove(&reconcile::sandbox_name(&name)).await {
            Ok(()) => {
                if ctx.store.delete_fleet_agent(&name)? {
                    ctx.store.record(&name, "deleted", "fleet")?;
                    println!("{name} removed");
                }
            }
            Err(e) => {
                eprintln!("{name}: {e:#}");
                failed = true;
            }
        }
    }
    for (name, entry) in desired {
        let digest = digests.remove(&name).expect("resolved above");
        let outcome = match ctx.store.get_agent(&name)? {
            None => {
                if let Some(workspace) = &entry.workspace {
                    ctx.store.ensure_workspace(workspace)?;
                }
                let agent = Agent::new(
                    name.clone(),
                    true,
                    AgentSpec {
                        owner: owner(),
                        role: entry.role,
                        role_digest: digest,
                        workspace: entry.workspace,
                        desired: Desired::Running,
                        env: entry.env,
                    },
                );
                ctx.store.insert_agent(&agent)?;
                ctx.store.record(&name, "created", &agent.spec.owner)?;
                "created"
            }
            Some(agent) if !agent.fleet => {
                eprintln!("{name}: exists but is not fleet-managed; skipping");
                failed = true;
                continue;
            }
            Some(agent) if agent.spec.workspace != entry.workspace => {
                eprintln!("{name}: workspace changes need `agent rm` first; skipping");
                failed = true;
                continue;
            }
            Some(agent)
                if agent.spec.role == entry.role
                    && agent.spec.role_digest == digest
                    && agent.spec.env == entry.env =>
            {
                "unchanged"
            }
            Some(agent) => {
                ctx.store.set_fleet_spec(
                    &name,
                    &entry.role,
                    &digest,
                    &entry.env,
                    agent.generation,
                )?;
                ctx.store.record(&name, "updated", digest.as_str())?;
                "updated"
            }
        };
        match reconcile::reconcile(&ctx.store, &ctx.secrets, &ctx.vmm, &name).await {
            Ok(agent) => println!("{name} {outcome} ({})", agent.status.lifecycle.label()),
            Err(e) => {
                eprintln!("{name}: {e:#}");
                failed = true;
            }
        }
    }
    if failed {
        bail!("some agents failed to converge");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_specs_parse() {
        let spec: PortSpec = "9119".parse().unwrap();
        assert_eq!((spec.local, spec.guest), (9119, 9119));
        let spec: PortSpec = "8080:9118".parse().unwrap();
        assert_eq!((spec.local, spec.guest), (8080, 9118));
        let spec: PortSpec = "0:80".parse().unwrap();
        assert_eq!((spec.local, spec.guest), (0, 80));
        assert!("".parse::<PortSpec>().is_err());
        assert!("x:80".parse::<PortSpec>().is_err());
        assert!("80:".parse::<PortSpec>().is_err());
        assert!("80:0".parse::<PortSpec>().is_err());
        assert!("0".parse::<PortSpec>().is_err());
        assert!("70000".parse::<PortSpec>().is_err());
    }

    #[test]
    fn absent_init_and_env_never_reach_the_digest() {
        let role = parse_role(
            r#"
version = 1
name = "plain"
image = "alpine"
resources = { vcpus = 1, memory-mib = 64 }
network = { egress = ["example.com"] }
"#,
        )
        .unwrap();
        let (_, definition) = digest_role(&role);
        assert!(!definition.contains(r#""init""#), "{definition}");
        assert!(!definition.contains(r#""env""#), "{definition}");
        assert!(!definition.contains(r#""expose""#), "{definition}");
    }

    #[test]
    fn env_pairs_parse() {
        let EnvPair(key, value) = "FOO=bar".parse().unwrap();
        assert_eq!((key.as_str(), value.as_str()), ("FOO", "bar"));
        let EnvPair(_, value) = "FOO=a=b".parse().unwrap();
        assert_eq!(value, "a=b");
        let EnvPair(_, value) = "FOO=".parse().unwrap();
        assert_eq!(value, "");
        assert!("FOO".parse::<EnvPair>().is_err());
        assert!("lower=x".parse::<EnvPair>().is_err());
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

        let agent = AgentRow {
            name: "echo-1".parse().unwrap(),
            role: "echo".parse().unwrap(),
            owner: "dmytro".to_owned(),
            desired: "running",
            state: "running",
            vm: None,
            synced: true,
            ports: BTreeMap::from([("ui".parse().unwrap(), 19007)]),
        };
        assert_eq!(
            serde_json::to_string(&agent).unwrap(),
            r#"{"name":"echo-1","role":"echo","owner":"dmytro","desired":"running","state":"running","vm":null,"synced":true,"ports":{"ui":19007}}"#
        );

        let event = store::Event {
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

        let digest = "0".repeat(64);
        let detail = AgentDetail {
            name: "echo-1".parse().unwrap(),
            role: "echo".parse().unwrap(),
            role_digest: digest.parse().unwrap(),
            owner: "dmytro".to_owned(),
            fleet: false,
            workspace: None,
            desired: "running",
            state: "failed",
            reason: Some("boom".to_owned()),
            generation: 2,
            applied_generation: 1,
            applied_digest: None,
            vm: Some("stopped"),
            sandbox: "reef-echo-1".to_owned(),
            ports: BTreeMap::new(),
            env: BTreeMap::from([("FOO".parse().unwrap(), "bar".to_owned())]),
        };
        assert_eq!(
            serde_json::to_string(&detail).unwrap(),
            format!(
                r#"{{"name":"echo-1","role":"echo","role_digest":"{digest}","owner":"dmytro","fleet":false,"workspace":null,"desired":"running","state":"failed","reason":"boom","generation":2,"applied_generation":1,"applied_digest":null,"vm":"stopped","sandbox":"reef-echo-1","ports":{{}},"env":{{"FOO":"bar"}}}}"#
            )
        );
    }
}
