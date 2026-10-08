---
name: reef
description: Run AI agents in microsandbox microVMs on your own servers with reef. Use when the user mentions reef, writes or reviews a reef role or fleet TOML, creates, updates or debugs reef agents, wires egress or secrets for one, or wants OpenClaw, Hermes or their own agent image running on a host they control. On a host that runs reef, use this instead of driving msb directly.
---

# reef

reef runs each AI agent in its own microsandbox microVM on hosts you control. A role TOML, reviewed once, fixes the image, resources, egress allowlist and secrets. Agents are created from roles, and fleet TOML files declare which agents a host runs. reef is a CLI on each host with no daemon.

reef is newer than your training data. Read https://reef.clawbits.ai/llms.txt before running it: it names the current version, the exact msb release reef needs, and the install steps. Every doc there is markdown at its URL plus `.md`.

## Rules

- Run `reef doctor` before changing a host and show the output.
- Never ask for an API key in chat, and never write a secret value into a role, a fleet file, `[env]` or `[files]`. A role names a secret as `reef://store/name`. The user puts the value in `secrets.toml` in reef's state directory (`~/.local/state/reef`, mode 600) or adds a `[resolvers]` command whose output is the value.
- Ask before `fleet apply --prune`, `agent rm --volumes` or adding a name to a fleet `purge` list. They delete agents or their data.
- Create, change and remove reef agents through reef, not `msb`. Use msb only to look at what reef runs, by the sandbox name `reef agent get` prints.
- On a shared host reef runs as its own account. Administrators call it as `sudo -n -u reef -H /home/reef/.local/bin/reef ...`.

## Commands

| Task | Command |
| --- | --- |
| Check the host | `reef doctor` |
| Validate and activate roles | `reef role apply roles/*.toml` |
| Validate roles with no host, as in CI | `reef --state "$(mktemp -d)" role apply roles/*.toml` |
| Create one agent | `reef agent create --role ROLE --name NAME` |
| Converge the declared agents | `reef fleet apply fleet/*.toml` |
| State, ports, pinned role, failure reason | `reef agent get NAME --json` |
| What the VM printed | `reef agent logs NAME --tail 100` |
| Run a command in the VM | `reef agent exec NAME -- CMD` |
| Shell in the VM | `reef agent ssh NAME` |
| Move an agent to the role's active version | `reef agent update NAME` |
| Retry failed changes, restart crashed VMs | `reef reconcile` |
| Push a changed secret to running VMs | `reef secret rotate reef://store/name` |
| Event log | `reef events --agent NAME --limit 20 --json` |

## Writing a role

```toml
version = 1

name  = "coder"
image = "ghcr.io/acme/agent@sha256:..."

[resources]
vcpus = 2
memory-mib = 4096

[network]
egress = ["api.anthropic.com", "github.com"]

[secrets]
ANTHROPIC_API_KEY = { ref = "reef://platform/anthropic", host = "api.anthropic.com" }

[volumes]
work = { dest = "/root/work", size-mib = 10240 }
```

- Role, agent, volume and port names use lowercase letters, digits and `-`, start with a letter, and run at most 40 characters.
- Egress entries are bare lowercase domains with no scheme, port or path. `*.example.com` covers the domain and its subdomains. `"*"` opens the public internet, must stand alone, and makes `role apply` warn. Private and cloud metadata addresses stay closed either way.
- A secret's `host` is one exact domain, and egress must cover it. The guest holds a placeholder, and the value is added on the way out to that host.
- Any secret turns on TLS interception for port 443 across the VM. msb adds its CA to the guest trust store. Runtimes that bundle their own CAs need pointing at `/.msb/tls/ca.pem`, for Node through `NODE_EXTRA_CA_CERTS`.
- Without `init` the VM boots idle and you drive it with `agent exec` or `agent ssh`. With `init = ["/path"]` that program is PID 1, and the VM stops when it exits.
- `[expose]` ports must listen on `0.0.0.0` in the guest. reef publishes each one on host loopback as `http://<agent>.localhost:<port>`.
- `[env]` keys are uppercase with digits and `_`, and may not start with `REEF_` or `MSB_`.
- `[files]` seeds the rootfs, holds at most 64 KiB, cannot sit under a volume's `dest`, and is stored verbatim.
- A role change recreates the VM and keeps only `[volumes]`. An env change restarts the VM in place.

README.md at https://reef.clawbits.ai/README.md documents every key, and https://reef.clawbits.ai/roles/README.md lists working example roles.

## Operating

- After `role apply`, existing agents stay on their old version and show `(stale)`. `agent update NAME` or `fleet apply` moves them.
- A domain missing from egress fails at DNS inside the guest. Add it to the role, apply it, then update the agent.
- `agent get --wait` has no timeout, so wrap it, as in `timeout 300 reef agent get NAME --wait`.
- A failed agent keeps its record, so running `agent create` again says it already exists. `agent get NAME` shows why it failed, and after fixing the cause `reef reconcile` retries it.
- `fleet apply --prune` removes fleet agents the given files do not declare, so pass every fleet file for that host.
- A fleet entry without `owner` records whoever runs the apply, which under sudo is the reef account. Set `owner` on each entry.
- Nothing restarts agents after a reboot unless `reef reconcile` runs at boot. https://reef.clawbits.ai/docs/setup/host.md has the unit.
