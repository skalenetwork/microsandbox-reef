mod msb;
mod reconcile;
mod secrets;
mod store;
mod vmm;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use reef_core::{
    Agent, AgentName, AgentSpec, AgentStatus, Desired, Digest, Lifecycle, Role, RoleName,
    WorkspaceName, parse_role,
};
use secrets::Secrets;
use sha2::{Digest as _, Sha256};
use std::path::PathBuf;
use store::Store;
use vmm::Vmm;

#[derive(Parser)]
#[command(name = "reef", version, about = "Declared agents, disposable microVMs")]
struct Cli {
    /// State directory (db + secrets.toml)
    #[arg(long, global = true, env = "REEF_STATE", value_name = "DIR")]
    state: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
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
    /// Check this host can run agents
    Doctor,
}

#[derive(Subcommand)]
enum RoleCommand {
    /// Validate and import role files, activating each new version
    Apply { files: Vec<PathBuf> },
    /// List roles and their active versions
    List,
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
    },
    /// List agents with their observed VM state
    List,
    /// Run a command inside an agent's VM
    Exec {
        name: AgentName,
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    /// Re-pin to the role's active version and recreate the VM
    Update { name: AgentName },
    /// Set desired state to running and reconcile
    Start { name: AgentName },
    /// Set desired state to stopped and reconcile
    Stop { name: AgentName },
    /// Destroy the VM and the record; workspaces survive
    Rm { name: AgentName },
    /// Show an agent's event history
    History { name: AgentName },
}

struct Ctx {
    store: Store,
    secrets: Secrets,
    vmm: msb::Msb,
}

impl Ctx {
    fn open(state: Option<PathBuf>) -> Result<Self> {
        let dir = state.map_or_else(default_state_dir, Ok)?;
        Ok(Self {
            store: Store::open(&dir.join("reef.db"))?,
            secrets: Secrets::load(&dir.join("secrets.toml"))?,
            vmm: msb::Msb::new(&dir)?,
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
    match cli.command {
        Command::Role { command } => role_command(Ctx::open(cli.state)?, command),
        Command::Agent { command } => agent_command(Ctx::open(cli.state)?, command).await,
        Command::Doctor => msb::doctor(),
    }
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
                            short(&digest)
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
        RoleCommand::List => {
            for (name, digest, image) in ctx.store.list_roles()? {
                println!("{name:24} {} {image}", &digest[..12]);
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
        } => {
            let (digest, _) = ctx
                .store
                .active_role(&role)?
                .with_context(|| format!("no such role: {role} (run `reef role apply` first)"))?;
            if let Some(workspace) = &workspace {
                ctx.store.ensure_workspace(workspace)?;
            }
            let agent = Agent {
                name: name.clone(),
                generation: 1,
                spec: AgentSpec {
                    owner: whoami(),
                    role,
                    role_digest: digest,
                    workspace,
                    desired: Desired::Running,
                },
                status: AgentStatus {
                    lifecycle: Lifecycle::Pending,
                    applied_generation: 0,
                    applied_digest: None,
                },
            };
            ctx.store.insert_agent(&agent)?;
            ctx.store.record(&name, "created", &agent.spec.owner)?;
            let agent = reconcile::reconcile(&ctx.store, &ctx.secrets, &ctx.vmm, &name).await?;
            println!("{} {}", agent.name, agent.status.lifecycle.label());
            Ok(())
        }
        AgentCommand::List => {
            println!(
                "{name:24} {role:16} {owner:10} {desired:8} {state:8} {vm:8} SYNC",
                name = "NAME",
                role = "ROLE",
                owner = "OWNER",
                desired = "DESIRED",
                state = "STATE",
                vm = "VM",
            );
            for agent in ctx.store.list_agents()? {
                let vm = ctx
                    .vmm
                    .status(&reconcile::sandbox_name(&agent.name))
                    .await?;
                let vm = match vm {
                    Some(reef_core::VmStatus::Running) => "running",
                    Some(reef_core::VmStatus::Stopped) => "stopped",
                    None => "-",
                };
                println!(
                    "{:24} {:16} {:10} {:8} {:8} {:8} {}",
                    agent.name.as_str(),
                    agent.spec.role.as_str(),
                    agent.spec.owner,
                    agent.spec.desired.as_str(),
                    agent.status.lifecycle.label(),
                    vm,
                    if agent.reconciled() { "yes" } else { "drift" },
                );
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
                    short(&active)
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
                short(&active)
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
        AgentCommand::History { name } => {
            for (at, kind, detail) in ctx.store.history(&name)? {
                println!("{at} {kind:8} {detail}");
            }
            Ok(())
        }
    }
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
    let mut hex = String::with_capacity(64);
    for byte in hash {
        hex.push_str(&format!("{byte:02x}"));
    }
    (hex.parse().expect("sha256 hex is a digest"), definition)
}

fn short(digest: &Digest) -> &str {
    &digest.as_str()[..12]
}

fn whoami() -> String {
    std::env::var("USER").unwrap_or_else(|_| "unknown".to_owned())
}
