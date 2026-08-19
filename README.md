# reef v2

Declared agents, disposable microVMs. An org describes agent **roles** as TOML
files; developers create **agents** from those roles; reef keeps each agent
materialized as a [microsandbox](https://github.com/superradcompany/microsandbox)
microVM that matches its record. The record is durable, the VM is cattle.

Design and invariants: [ARCHITECTURE.md](ARCHITECTURE.md).

## Layout

| Crate | What | Depends on |
|---|---|---|
| `reef-core` | Domain types, role-file parsing, the pure `plan()` reconcile function | serde, toml — no I/O |
| `reef` | The binary: CLI, SQLite store, secrets file, microsandbox adapter | reef-core, rusqlite, microsandbox |

## Use

```sh
reef doctor                                  # can this host run agents?
reef role apply roles/*.toml                 # validate + import, from CI or by hand
reef agent create --role code-reviewer --name reviewer-1
reef agent list
reef agent exec reviewer-1 -- echo hi
reef agent update reviewer-1                 # re-pin to the role's active version
reef agent stop reviewer-1
reef agent start reviewer-1
reef agent rm reviewer-1                     # VM destroyed, workspace kept
reef agent history reviewer-1
```

Every mutating command reconciles inline: the CLI returns when the VM matches
the record. There is no daemon; VMs are created detached and outlive reef.

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

`network.egress` is required: agents get deny-by-default egress, and the list
is domains only (the allowlist is enforced at DNS, so group rules like
"public" do not resolve). A wildcard `*.x` covers `x` and its subdomains —
matching what the runtime enforces. Secrets bind to the one host they may be
sent to; the VM only ever sees a placeholder — the real value is substituted
host-side by microsandbox's proxy and never enters the guest.

## State

`$XDG_STATE_HOME/reef` (default `~/.local/state/reef`), overridable with
`--state` / `REEF_STATE`:

- `reef.db` — roles, agents, workspaces, events (SQLite, WAL). Desired state
  plus the last applied status; VM liveness is re-derived from the runtime on
  every command. Sandboxes are labeled with this state dir's id, and reef
  refuses to destroy a sandbox another state dir (or you, by hand) created.
- `secrets.toml` — resolves `reef://store/name` references. Must be mode 0600
  or reef refuses to read it. A store is either an inline table (**plaintext
  at rest**) or, under `[resolvers]`, a command run at VM create whose stdout
  is the value — which plugs reef into whatever the org already uses (the CLI
  authenticates however the host already does; reef holds no credential for
  the credential store):

  ```toml
  [resolvers]
  op  = "op read 'op://Infra/{name}/credential' -n"
  bao = "bao kv get -field=value -mount=secret 'reef/{name}'"
  aws = "aws secretsmanager get-secret-value --secret-id '{name}' --query SecretString --output text"

  [local]
  demo = "sk-demo-not-real"
  ```

  An inline value wins over a resolver for the same store. Substituting
  `{name}` is injection-safe by construction: ref names are `[a-z0-9-]`, max
  40 chars, validated at parse.

## Known limits (v1)

- Secrets are plaintext at rest on the reef host, in two places: `secrets.toml`
  (0600 or reef refuses to read it) and microsandbox's own sandbox config under
  `~/.microsandbox`, which stores each injected value verbatim until the VM is
  recreated. Editing `secrets.toml` alone does not refresh a running agent —
  recreate it (`agent update` after a role change, or `rm` + `create`). `reef
  doctor` warns when `~/.microsandbox` is readable by other users.
- No disk I/O throttling — microsandbox caps disk size, not IOPS.
- One host, no auth on the CLI (it is a local tool; the HTTP API comes later
  and will not ship without auth).
- `microsandbox` is pinned exactly (`=0.6.9`, beta upstream); upgrades are a
  deliberate task, never a routine bump.

## Test

```sh
cargo test                    # pure + fake-backed, no VMM needed
cargo test -- --ignored       # boots a real microVM (needs msb + KVM/HVF)
```
