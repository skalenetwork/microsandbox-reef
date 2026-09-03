<div align="center">

<img src="site/src/assets/reef.png" alt="" width="88" height="88">

# reef

**Run OpenClaw agents from one reviewed file.**

[Install](#install) · [Commands](#commands) · [Docs](https://reef.clawbits.ai/docs) · [Architecture](ARCHITECTURE.md)

</div>

A **role** is a TOML file: the image, the resources, the domains an agent may
reach, and the secrets it may spend. Developers create **agents** from the
roles you approved, one command each, and every agent runs in its own
[microsandbox](https://github.com/superradcompany/microsandbox) microVM on
your own servers. There is no daemon: every mutating command reconciles
inline, and the VMs outlive reef.

## Install

```sh
curl -fsSL https://reef.clawbits.ai/install | sh
```

Latest release to `~/.local/bin`, no sudo; `REEF_INSTALL` overrides the
directory, `REEF_VERSION=0.4.0` pins a version. Linux x86_64/aarch64 and
Apple Silicon macOS; the Linux builds are glibc and need 2.39 or newer.

reef drives microsandbox rather than shipping it, so a host that never builds
reef from source needs the `msb` bundle installed once. The installer always
takes the newest release, which is not the one reef is pinned to, so pin it:

```sh
curl -fsSL https://install.microsandbox.dev | sh
msb self downgrade 0.6.16 -y
msb doctor
```

`msb doctor` is what checks the host can run microVMs: CPU virtualization, the
KVM device, and whether this account can open it. `reef doctor` reports the msb
it resolved, and that version has to be the pinned one or `agent create` fails
on a launch-config mismatch.

`reef update` replaces the binary in place with the latest release. Commands
note a newer version on stderr, checked at most once a day; `REEF_NO_UPDATE_CHECK=1`
turns the notice off.

## Run OpenClaw in a microVM

```sh
curl -fsSL https://reef.clawbits.ai/roles/openclaw.toml -o role.toml
curl -fsSL https://reef.clawbits.ai/fleet/openclaw.toml -o fleet.toml
# put your own OPENCLAW_GATEWAY_TOKEN in fleet.toml: openssl rand -hex 32
reef role apply role.toml
reef fleet apply fleet.toml
```

No secrets file and nothing configured in advance: open the URL `fleet apply`
printed, paste the token, pick a model provider in the browser.

`role apply` says what the file costs:

```
warn   openclaw disables egress filtering; its agents reach any host
```

That is this role's `egress = ["*"]`, and it is deliberate: an agent that
browses the web has to reach the web. Opting out takes writing that rule, and
reef warns on every apply. Either way the file is the whole policy.

## Narrow it

Same shape with the allowlist kept. These four lines of
[roles/hermes.toml](roles/hermes.toml) are a
[Hermes](https://github.com/NousResearch/hermes-agent) agent's entire blast
radius:

```toml
[network]
egress = ["openrouter.ai"]

[secrets]
OPENROUTER_API_KEY = { ref = "reef://hermes/openrouter", host = "openrouter.ai" }
```

One domain reachable, one key spendable against that domain and nowhere else;
the guest sees a placeholder, never the value. The rest of the file is the
image, resources, the dashboard port and a volume: twenty lines in all.
Approve that version once and every agent created from it inherits the policy,
while agents left on an older version read as stale.

Put the key in `~/.local/state/reef/secrets.toml` (`chmod 600`):

```toml
[hermes]
openrouter = "sk-or-..."
```

```sh
reef role apply roles/hermes.toml
reef fleet apply fleet/hermes.toml
reef agent get bob-hermes
```

Two agents, each with its dashboard on the `ports` line of `agent get` - log
in as `ana` or `bob`, password `password`.

## What reef adds to microsandbox

The isolation is
[microsandbox](https://github.com/superradcompany/microsandbox): the microVM,
the DNS-enforced egress allowlist, and the host-side substitution that keeps a
secret value out of the guest are its features, reached through one module.
reef adds the layer above them: the role file that settles all three before an
agent exists, one reviewed version shared by every agent created from it, a
stale mark when an agent is left behind, fleet convergence, agent records that
outlive the VMs, and one console across hosts.

## Commands

| Command | |
| --- | --- |
| `reef doctor` | Can this host run agents? |
| `reef role apply roles/*.toml` | Validate and import, from CI or by hand |
| `reef role list` | Roles and their active versions |
| `reef role get code-reviewer` | One role's active definition and the agents on it |
| `reef agent create --role code-reviewer --name reviewer-1` | Create an agent and reconcile it |
| `reef agent list` | Every agent with its observed VM state |
| `reef agent get reviewer-1 --wait` | One agent in detail; `--wait` blocks until settled |
| `reef agent exec reviewer-1 -- echo hi` | Run a command inside the VM |
| `reef agent ssh reviewer-1` | Interactive terminal in the VM |
| `reef agent forward reviewer-1` | No ports: list what the VM is listening on |
| `reef agent forward reviewer-1 9119` | Tunnel `reviewer-1.localhost:9119` into the VM until Ctrl-C |
| `reef agent update reviewer-1` | Re-pin to the role's active version |
| `reef agent stop reviewer-1` | Desired state stopped |
| `reef agent start reviewer-1` | Desired state running |
| `reef agent rm reviewer-1` | VM destroyed, volumes kept |
| `reef fleet apply fleet/*.toml` | Converge the declared fleet |
| `reef events --agent reviewer-1` | The event log, oldest first |
| `reef ui prod-eu prod-us` | Console: watch and drive agents here or on ssh hosts |

`role list`, `role get`, `agent list`, `agent get`, and `events` take
`--json`.

`agent get` reads an agent's whole blast radius off the role version it is
pinned to: image, resources, egress and secret bindings (the `reef://` ref and
its host, never a value). Both commands mark that pin against the role's active
version - `(stale)` on the role line, `role_current` in JSON - so an agent left
behind by a `role apply` is visible without comparing digests by hand.

`agent get --wait` polls until the agent settles - reconciled and in its
desired state - or reports failed, which exits nonzero. It has no timeout and
nothing reconciles while it waits; in scripts, wrap it in `timeout(1)`.

`events` prints the log oldest-first; `--after ID` returns only what is newer,
so a collector can poll it without re-reading. `agent get` prints the VM's
`sandbox` name - the handle for the runtime's own tools, such as
`msb logs <sandbox>` for captured guest output.

### Forwarding

`agent forward` with no ports reads the guest's `/proc/net/tcp` and lists the
ports it is listening on that are reachable from the guest's loopback, so every
port it names is one you can actually forward. It binds host loopback only and
tunnels through the guest agent channel - it reaches services on the guest's
own loopback and publishes nothing at the VM boundary. Like `exec`, it is
operator access: the role's egress list stays the agent's entire network policy.

### Terminal access

`agent ssh` drops you into an interactive shell in the VM over microsandbox's
SSH bridge - local operator access like `exec`, no identity involved. Authorize
your key once with `msb ssh authorize --file ~/.ssh/id_ed25519.pub`.

`agent serve` bridges one SSH session into an agent and is meant to run as an
sshd `ForceCommand`: it reads the client's CA-signed certificate from
`SSH_USER_AUTH`, the requested agent name from `SSH_ORIGINAL_COMMAND`, admits
the caller only if a certificate principal matches the agent's `owner` (set
with `--owner` at create, or per agent in a fleet file; default `$USER`),
records a `served` event, and hands the session to `msb ssh serve --stdio`.
The full pattern - certificates, sshd config, client config - is
[enterprise terminal access](https://reef.clawbits.ai/docs/enterprise/access).

### Console

`reef ui` is a full-screen view of every agent on this host: state, VM, drift
and ports in one table, Enter for what `agent get` prints, and `s`, `x`, `u`,
`d` to start, stop, update and remove the selected agent (update and remove
ask first). Tab switches to the roles table - active version, image, and how
many agents run each role and how many are stale - where Enter prints what
`role get` prints. Roles are read-only there; `role apply` stays a CLI
command, since it takes files. It is a client of the `--json` commands above and never opens the
state directory itself: locally it runs this binary, and given ssh host aliases
it runs `ssh ALIAS ~/.local/bin/reef ...` on each and merges the tables:

```sh
reef ui prod-eu prod-us
```

Each alias is a `Host` in `~/.ssh/config`, so bastions, certificates and key
agents work as they do for `ssh` itself. Connect to a new host once in a
terminal first: the console never answers prompts. `--reef CMD` names the
command that runs reef on the hosts when it is not `~/.local/bin/reef`, such
as `--reef 'sudo -n -u reef -H /home/reef/.local/bin/reef'` on a host set up
for [remote access](https://reef.clawbits.ai/docs/enterprise/access). The table
refreshes every five seconds and after each action; `ControlMaster auto`,
`ControlPath ~/.ssh/cm-%C` and `ControlPersist 600` on the host's block keep
one connection open between polls. The detail view prints the `ssh -L` and `agent ssh` lines that reach an
agent's ports and terminal from your laptop.

Color marks status only: green for running, yellow for pending, for drift and
for an action in flight, red for failed and for a host that will not answer.
Names, images and ports stay plain and metadata stays dim, so a healthy fleet
reads quietly. Setting `NO_COLOR` drops the color and marks failures bold
instead.

## Roles

```toml
version = 1

name  = "code-reviewer"
image = "ghcr.io/acme/agent@sha256:…"

[resources]
vcpus = 2
memory-mib = 4096

[network]
egress = ["api.anthropic.com", "github.com"]

[secrets]
ANTHROPIC_API_KEY = { ref = "reef://platform/anthropic", host = "api.anthropic.com" }
```

### Init

`init` (optional, exec-form: `init = ["/init"]`) names the program that becomes
the VM's PID 1 - how service images (s6-overlay, systemd, entrypoint scripts)
boot. The guest agent survives the handoff as its child, so `exec` and
`forward` keep working; when the init exits, the VM stops. Absent, the VM
boots idle and is driven via `exec`.

### Env

Env is layered: the image's own `ENV` is the base, the role's `[env]` overrides
it key by key, and `agent create --env KEY=VALUE` overrides both for that one
agent (kept in the record, surviving recreates; shown by `agent get`). Keys are
`UPPER_SNAKE`. Changing an agent's env does not rebuild it: reef applies the
change to the existing VM and restarts it, so anything written to the rootfs
survives. Only a role change recreates the VM, and a recreate keeps only what
`[volumes]` declares. Secrets never go in any env layer - values are visible
verbatim in the guest; derived material like a password hash is fine.

### Files

`[files]` seeds the rootfs before the VM starts, so a role carries the agent's
configuration and not only its containment:

```toml
[files]
"/etc/agent/config.json" = '''
{ "model": "claude-sonnet-4-6" }
'''
```

Paths are absolute and their parents are created. The role's copy replaces
whatever the image shipped there, the same way `[env]` overrides the image's
own `ENV`, and a file edit is a role change like any other - it recreates the
VM. Write the small override layer an app already reads, not a copy of its
config: the whole table is capped at 64 KiB. A path inside a `[volumes]` dest
is a parse error: the volume mounts over it at start. Content is part of the
role, stored verbatim in `reef.db` and never substituted - credentials go in
`[secrets]`.

A file is root-owned and world-readable unless it names its own mode:

```toml
"/opt/start" = { content = "#!/bin/sh\nexec /app/serve\n", mode = 0o755 }
```

### Ports

`[expose]` names the guest ports a role serves (`ui = 9119`; they must listen
on `0.0.0.0` in the guest). For each entry reef allocates the agent a stable
host port from `19000-19999` at create, binds it to loopback, and keeps it for
the agent's life; `agent rm` releases it. `create`, `start`, and `get` print
each as `http://<agent>.localhost:<port>`, as does `fleet apply` for what it
creates - every name under `.localhost` resolves to loopback, so an agent's
pages get their own browser origin instead of sharing one `127.0.0.1` jar.
The `ports` maps in `agent list --json` / `get --json` are the handoff to
whatever ingress the org already runs - reef does no TLS, auth, or routing
itself.

The guest is told its own name as `REEF_AGENT` and its published ports as
`REEF_PORT_<NAME>` (`control-ui` becomes `REEF_PORT_CONTROL_UI`) - between
them, its own URL, which reef picks and the guest cannot otherwise know. The
`REEF_` prefix is reserved: role `[env]` and `--env` reject it, so the
namespace is always reef's.

### Volumes

`[volumes]` declares the guest paths whose contents must outlive the VM. Each
entry gets one volume per agent, named `reef-vol-<agent>-<entry>`, created at
first use with `size-mib` as an enforced quota:

```toml
[volumes]
data = { dest = "/opt/data", size-mib = 10240 }
```

A volume survives stop/start, a role change (which recreates the VM), and
`agent rm`; `agent get` prints its name so `msb volume rm <name>` can delete
it. Everything outside a declared path lives in the rootfs and is replaced
whenever the role changes - that is what an image upgrade *is*. reef cannot
persist state an image neither declares nor rebuilds on its own: check where
your image keeps state (`msb image inspect <image>` shows its OCI config) and
declare those paths.

### Egress and secrets

`network.egress` is required: agents get deny-by-default egress, and the list
is domains only (the allowlist is enforced at DNS). A wildcard `*.x` covers
`x` and its subdomains, and a raw-IP connection is allowed only while a live
DNS answer for an allowed domain pins that IP (pins last the record's TTL).
The list covers the internet, never the host: an agent cannot reach the reef
host's loopback or another agent's published port, and `"*"` does not change
that.
Secrets bind to the one host they may be sent to; the VM only ever sees a
placeholder - the real value is substituted host-side by microsandbox's proxy
and never enters the guest.

## Fleets

Declare the org's agents and converge with `reef fleet apply fleet/*.toml`:
listed agents are created, recreated when their role drifts, and restarted
in place when only their env drifts.

```toml
version = 1

[agents.ana-hermes]
role = "hermes"
owner = "ana"
env = { HERMES_DASHBOARD_BASIC_AUTH_USERNAME = "ana" }
```

`owner` names who `agent serve` admits; omitted, an existing agent keeps its
owner and a new one records the applying user.

Removal is opt-in: `--prune` also deletes fleet agents the given files no
longer declare, so pass it the whole fleet directory - a partial file list
with `--prune` deletes everything it cannot see. Without the flag, undeclared
agents are only reported. Hand-made agents are never touched, and volumes
survive removal.

## State

`$XDG_STATE_HOME/reef` (default `~/.local/state/reef`), overridable with
`--state` / `REEF_STATE`:

- `reef.db` - roles, agents, ports, events (SQLite, WAL). Desired state
  plus the last applied status; VM liveness is re-read from the runtime on
  every command.
- `secrets.toml` - resolves `reef://store/name` references; `chmod 600` or
  reef refuses to read it. A store is an inline table (**plaintext at rest**)
  or, under `[resolvers]`, a command run at VM create whose stdout is the
  value - plugging reef into whatever the org already runs:

  ```toml
  [resolvers]
  op = "op read 'op://Infra/{name}/credential' -n"

  [local]
  demo = "sk-demo-not-real"
  ```

  An inline value wins over a resolver for the same store. `{name}` is
  injection-safe by construction: ref names are `[a-z0-9-]`, max 40 chars,
  validated at parse.

## Limits

- Secrets are plaintext at rest in two places: `secrets.toml` (0600-guarded)
  and microsandbox's sandbox config under `~/.microsandbox` until the VM is
  recreated - editing `secrets.toml` alone does not refresh a running agent.
  `reef doctor` warns when `~/.microsandbox` is readable by other users.
- Published host ports are unique per state dir only: a second `--state` on
  this host, or an unrelated process squatting `19000-19999`, can collide -
  and microsandbox reports a failed port bind only in its own logs.
- `<agent>.localhost` is a name, not a route: it resolves to `127.0.0.1` on
  macOS and on Linux with systemd-resolved, and nowhere else - a minimal glibc
  or musl host resolves only `localhost`, where the same port still answers on
  `127.0.0.1`. `reef doctor` says which host you are on.
- No disk I/O throttling - microsandbox caps disk size, not IOPS.
- One host per state dir, no auth on the CLI: it runs where the state lives,
  and `reef ui` reaches it over your own ssh (the HTTP API comes later and will
  not ship without auth).
- `microsandbox` is pinned exactly (`=0.6.16`, beta upstream); upgrades are a
  deliberate task, never a routine bump. reef migrates `~/.microsandbox` to
  that schema on first run, and an older `msb` refuses the store afterwards -
  upgrade `msb` alongside reef, or roll back with `msb self downgrade`.

## Tests

```sh
cargo test                    # pure + fake-backed, no VMM needed
cargo test -- --ignored       # boots a real microVM (needs msb + KVM/HVF)
```

---

<div align="center">

MIT License. © SKALE Labs.

</div>
