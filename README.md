# reef

Declared agents, disposable microVMs. An org describes agent **roles** as TOML
files; developers create **agents** from those roles; reef keeps each agent
materialized as a [microsandbox](https://github.com/superradcompany/microsandbox)
microVM that matches its record. The record is durable, the VM is cattle; there
is no daemon — every mutating command reconciles inline, and VMs outlive reef.

Design and invariants: [ARCHITECTURE.md](ARCHITECTURE.md).

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/skalenetwork/reef/main/install.sh | sh
```

Latest release to `~/.local/bin`, no sudo; `REEF_INSTALL` overrides the
directory, `REEF_VERSION=0.1.0` pins a version. Linux x86_64/aarch64 and
Apple Silicon macOS. Then: `reef doctor`.

## Use

```sh
reef doctor                                  # can this host run agents?
reef role apply roles/*.toml                 # validate + import, from CI or by hand
reef agent create --role code-reviewer --name reviewer-1
reef agent list
reef agent get reviewer-1 --wait             # one agent in detail; --wait blocks until settled
reef agent exec reviewer-1 -- echo hi
reef agent forward reviewer-1                # no ports: list what the VM is listening on
reef agent forward reviewer-1 9119           # tunnel 127.0.0.1:9119 into the VM until Ctrl-C
reef agent update reviewer-1                 # re-pin to the role's active version
reef agent stop reviewer-1
reef agent start reviewer-1
reef agent rm reviewer-1                     # VM destroyed, workspace kept
reef agent history reviewer-1
```

`role list`, `agent list`, `agent get`, and `agent history` take `--json`.
`agent get --wait` polls until the agent settles — reconciled and in its
desired state — or reports failed, which exits nonzero. It has no timeout and
nothing reconciles while it waits; in scripts, wrap it in `timeout(1)`.

`agent forward` with no ports reads the guest's `/proc/net/tcp` and lists the
ports it is listening on that are reachable from the guest's loopback, so every
port it names is one you can actually forward. It binds host loopback only and
tunnels through the guest agent channel — it reaches services on the guest's own loopback and publishes
nothing at the VM boundary. Like `exec`, it is operator access: the role's
egress list stays the agent's entire network policy.

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
the VM's PID 1 — how service images (s6-overlay, systemd, entrypoint scripts)
boot. The guest agent survives the handoff as its child, so `exec` and
`forward` keep working; when the init exits, the VM stops. Absent, the VM
boots idle and is driven via `exec`.

`[env]` sets plain environment variables for every process in the VM,
overriding the image's own `ENV` key by key (keys are `UPPER_SNAKE`). Secrets
never go here — an `[env]` value is visible verbatim in the guest.

`[expose]` names the guest ports a role serves (`ui = 9119`; they must listen
on `0.0.0.0` in the guest). For each entry reef allocates the agent a stable
host port from `19000-19999` at create, binds it to loopback, and keeps it for
the agent's life; `agent rm` releases it. The `ports` maps in `agent list
--json` / `get --json` are the handoff to whatever ingress the org already
runs — reef does no TLS, auth, or routing itself.

`network.egress` is required: agents get deny-by-default egress, and the list
is domains only (the allowlist is enforced at DNS). A wildcard `*.x` covers
`x` and its subdomains, and a raw-IP connection is allowed only while a live
DNS answer for an allowed domain pins that IP (pins last the record's TTL).
Secrets bind to the one host they may be sent to; the VM only ever sees a
placeholder — the real value is substituted host-side by microsandbox's proxy
and never enters the guest.

## State

`$XDG_STATE_HOME/reef` (default `~/.local/state/reef`), overridable with
`--state` / `REEF_STATE`:

- `reef.db` — roles, agents, workspaces, events (SQLite, WAL). Desired state
  plus the last applied status; VM liveness is re-read from the runtime on
  every command.
- `secrets.toml` — resolves `reef://store/name` references; mode 0600 or reef
  refuses to read it. A store is an inline table (**plaintext at rest**) or,
  under `[resolvers]`, a command run at VM create whose stdout is the value —
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
  recreated — editing `secrets.toml` alone does not refresh a running agent.
  `reef doctor` warns when `~/.microsandbox` is readable by other users.
- Published host ports are unique per state dir only: a second `--state` on
  this host, or an unrelated process squatting `19000-19999`, can collide —
  and microsandbox reports a failed port bind only in its own logs.
- No disk I/O throttling — microsandbox caps disk size, not IOPS.
- One host, no auth on the CLI (a local tool; the HTTP API comes later and
  will not ship without auth).
- `microsandbox` is pinned exactly (`=0.6.10`, beta upstream); upgrades are a
  deliberate task, never a routine bump.

## Test

```sh
cargo test                    # pure + fake-backed, no VMM needed
cargo test -- --ignored       # boots a real microVM (needs msb + KVM/HVF)
```
