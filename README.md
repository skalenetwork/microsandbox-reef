# reef

Isolated computers for agents. An org describes agent **roles** as TOML
files; developers create **agents** from those roles; reef keeps each agent
materialized as a [microsandbox](https://github.com/superradcompany/microsandbox)
microVM that matches its record. The record is durable, the VM is cattle; there
is no daemon - every mutating command reconciles inline, and VMs outlive reef.

Design and invariants: [ARCHITECTURE.md](ARCHITECTURE.md).

## Install

```sh
curl -fsSL https://reef.clawbits.ai/install | sh
```

Latest release to `~/.local/bin`, no sudo; `REEF_INSTALL` overrides the
directory, `REEF_VERSION=0.1.0` pins a version. Linux x86_64/aarch64 and
Apple Silicon macOS. Then: `reef doctor`.

`reef update` replaces the binary in place with the latest release. Commands
note a newer version on stderr, checked at most once a day; `REEF_NO_UPDATE_CHECK=1`
turns the notice off.

## Use

```sh
reef doctor                                  # can this host run agents?
reef role apply roles/*.toml                 # validate + import, from CI or by hand
reef agent create --role code-reviewer --name reviewer-1
reef agent list
reef agent get reviewer-1 --wait             # one agent in detail; --wait blocks until settled
reef agent exec reviewer-1 -- echo hi
reef agent ssh reviewer-1                    # interactive terminal in the VM
reef agent forward reviewer-1                # no ports: list what the VM is listening on
reef agent forward reviewer-1 9119           # tunnel reviewer-1.localhost:9119 into the VM until Ctrl-C
reef agent update reviewer-1                 # re-pin to the role's active version
reef agent stop reviewer-1
reef agent start reviewer-1
reef agent rm reviewer-1                     # VM destroyed, volumes kept
reef events --agent reviewer-1               # the event log, oldest first
```

`role list`, `agent list`, `agent get`, and `events` take `--json`.
`agent get --wait` polls until the agent settles - reconciled and in its
desired state - or reports failed, which exits nonzero. It has no timeout and
nothing reconciles while it waits; in scripts, wrap it in `timeout(1)`.

`events` prints the log oldest-first; `--after ID` returns only what is newer,
so a collector can poll it without re-reading. `agent get` prints the VM's
`sandbox` name - the handle for the runtime's own tools, such as
`msb logs <sandbox>` for captured guest output.

`agent forward` with no ports reads the guest's `/proc/net/tcp` and lists the
ports it is listening on that are reachable from the guest's loopback, so every
port it names is one you can actually forward. It binds host loopback only and
tunnels through the guest agent channel - it reaches services on the guest's own loopback and publishes
nothing at the VM boundary. Like `exec`, it is operator access: the role's
egress list stays the agent's entire network policy.

`agent ssh` drops you into an interactive shell in the VM over microsandbox's
SSH bridge - local operator access like `exec`, no identity involved. Authorize
your key once with `msb ssh authorize --file ~/.ssh/id_ed25519.pub`. For remote,
identity-gated access, `agent serve` is the counterpart below.

`agent serve` bridges one SSH session into an agent and is meant to run as an
sshd `ForceCommand`: it reads the client's CA-signed certificate from
`SSH_USER_AUTH`, the requested agent name from `SSH_ORIGINAL_COMMAND`, admits
the caller only if a certificate principal matches the agent's `owner` (set
with `--owner` at create, or per agent in a fleet file; default `$USER`),
records a `served` event, and hands the session to `msb ssh serve --stdio`.
The full pattern - certificates, sshd config, client config - is
[enterprise terminal access](https://reef.clawbits.ai/docs/enterprise/access).

## A role file

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

`init` (optional, exec-form: `init = ["/init"]`) names the program that becomes
the VM's PID 1 - how service images (s6-overlay, systemd, entrypoint scripts)
boot. The guest agent survives the handoff as its child, so `exec` and
`forward` keep working; when the init exits, the VM stops. Absent, the VM
boots idle and is driven via `exec`.

Env is layered: the image's own `ENV` is the base, the role's `[env]` overrides
it key by key, and `agent create --env KEY=VALUE` overrides both for that one
agent (kept in the record, surviving recreates; shown by `agent get`). Keys are
`UPPER_SNAKE`. Changing an agent's env does not rebuild it: reef applies the
change to the existing VM and restarts it, so anything written to the rootfs
survives. Only a role change recreates the VM, and a recreate keeps only what
`[volumes]` declares. Secrets never go in any env layer - values are visible verbatim
in the guest; derived material like a password hash is fine.

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

`network.egress` is required: agents get deny-by-default egress, and the list
is domains only (the allowlist is enforced at DNS). A wildcard `*.x` covers
`x` and its subdomains, and a raw-IP connection is allowed only while a live
DNS answer for an allowed domain pins that IP (pins last the record's TTL).
Secrets bind to the one host they may be sent to; the VM only ever sees a
placeholder - the real value is substituted host-side by microsandbox's proxy
and never enters the guest.

## A fleet file

Declare the org's agents and converge with `reef fleet apply fleet/*.toml`:
listed agents are created, recreated when their role drifts, and restarted
in place when only their env drifts.
Removal is opt-in: `--prune` also deletes fleet agents the given files no
longer declare, so pass it the whole fleet directory - a partial file list
with `--prune` deletes everything it cannot see. Without the flag, undeclared
agents are only reported. Hand-made agents are never touched, and volumes
survive removal.

```toml
version = 1

[agents.ana-hermes]
role = "hermes"
owner = "ana"
env = { HERMES_DASHBOARD_BASIC_AUTH_USERNAME = "ana" }
```

`owner` names who `agent serve` admits; omitted, an existing agent keeps its
owner and a new one records the applying user.

## Try it: a hermes fleet

Put an OpenRouter key in `~/.local/state/reef/secrets.toml` (mode 0600):

```toml
[hermes]
openrouter = "sk-or-..."
```

```sh
reef role apply roles/hermes.toml
reef fleet apply fleet/hermes.toml
reef agent get bob-hermes
```

Two [Hermes](https://github.com/NousResearch/hermes-agent) agents, each with
its dashboard on the `ports` line of `agent get` - log in as `ana` or `bob`,
password `password`.

## State

`$XDG_STATE_HOME/reef` (default `~/.local/state/reef`), overridable with
`--state` / `REEF_STATE`:

- `reef.db` - roles, agents, ports, events (SQLite, WAL). Desired state
  plus the last applied status; VM liveness is re-read from the runtime on
  every command.
- `secrets.toml` - resolves `reef://store/name` references; mode 0600 or reef
  refuses to read it. A store is an inline table (**plaintext at rest**) or,
  under `[resolvers]`, a command run at VM create whose stdout is the value -
  plugging reef into whatever the org already runs:

  ```toml
  [resolvers]
  op = "op read 'op://Infra/{name}/credential' -n"

  [local]
  demo = "sk-demo-not-real"
  ```

  An inline value wins over a resolver for the same store. `{name}` is
  injection-safe by construction: ref names are `[a-z0-9-]`, max 40 chars,
  validated at parse.

## Known limits

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
- One host, no auth on the CLI (a local tool; the HTTP API comes later and
  will not ship without auth).
- `microsandbox` is pinned exactly (`=0.6.15`, beta upstream); upgrades are a
  deliberate task, never a routine bump. reef migrates `~/.microsandbox` to
  that schema on first run, and an older `msb` refuses the store afterwards -
  upgrade `msb` alongside reef, or roll back with `msb self downgrade`.

## Test

```sh
cargo test                    # pure + fake-backed, no VMM needed
cargo test -- --ignored       # boots a real microVM (needs msb + KVM/HVF)
```
