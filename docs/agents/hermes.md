# Hermes

Give each person their own
[Hermes](https://github.com/NousResearch/hermes-agent) agent: one microVM
apiece, a private dashboard, and nothing reachable on the network but
`openrouter.ai`. The role pins v0.21.0 by digest; copy it into your own repo
before you rely on it.

## Secret

The role spends one OpenRouter key. Put it in
`~/.local/state/reef/secrets.toml`, which must be `chmod 600`:

```toml
[hermes]
openrouter = "sk-or-..."
```

The value is substituted host-side against `openrouter.ai`. The guest only
ever sees a placeholder.

OpenRouter is only what this example picks. Any provider the agent supports
works: rename the key in `[secrets]`, point its `host` at that provider's API,
and put the same domain in the role's `[network] egress`.

## Fleet

One entry per person. `owner` is who may open a terminal into the agent
later; the env keys are the dashboard's basic auth, which every agent must
set or the dashboard fails closed:

```toml
[agents.ana-hermes]
role = "hermes"
owner = "ana"

[agents.ana-hermes.env]
HERMES_DASHBOARD_BASIC_AUTH_USERNAME = "ana"
HERMES_DASHBOARD_BASIC_AUTH_PASSWORD_HASH = "scrypt$16384$8$1$..."
HERMES_DASHBOARD_BASIC_AUTH_SECRET = "..."
```

The hash is scrypt; the secret is the HMAC key that signs sessions, and
without it every restart logs the agent's users out. Rotate a password with
the image's own helper:

```sh
reef agent exec ana-hermes -- env PYTHONPATH=/opt/hermes \
  /opt/hermes/.venv/bin/python -c \
  "import plugins.dashboard_auth.basic as b; print(b.hash_password('pw'))"
```

For org SSO instead, set `HERMES_DASHBOARD_OIDC_ISSUER` and
`HERMES_DASHBOARD_OIDC_CLIENT_ID` in the role's `[env]` and leave the basic
auth keys out.

## Run

```sh
curl -fsSL https://reef.clawbits.ai/roles/hermes.toml -o role.toml
curl -fsSL https://reef.clawbits.ai/fleet/hermes.toml -o fleet.toml
reef role apply role.toml
reef fleet apply fleet.toml
```

`fleet apply` prints each agent's URL as it creates it, and `agent get` shows
it again later. Open ana's and log in as `ana`. The shipped fleet file is a
demo: its hashes are the password `password` and its session secrets are in
public git, so replace both before this reaches anyone.

Re-run `reef fleet apply` after editing the file. An env change restarts the
agent in place; only a role change recreates the VM.

## Notes

- **`/opt/data` is the only path that outlives a recreate.** Config,
  sessions, memories, skills, cron jobs, notepads and the whole profile tree
  live there. Everything else is rootfs and is replaced by the next image.
- **The gateway is what keeps the VM up and fires cron.** `init` passes
  `gateway run`, so scheduled jobs and their `--continuity` carry-over tick
  without anyone opening the dashboard. With no arguments the image's main
  program is the interactive CLI, which exits, and the VM stops with it.
- **Agents cannot message each other.** `hermes peer` is an outbound call to
  another gateway's API server, and reef denies every agent the host and its
  neighbours. Bot-to-bot chat works between profiles inside one VM; across
  VMs it does not.
- **Some of v0.21.0 is desktop-only.** Bot Mode's roster and group chats, the
  MCP command center's health checks and cost overlays, and the agent-driven
  browser all need the Electron app. The dashboard still serves chat,
  sessions, cron, skills, MCP server management and a terminal into the real
  TUI.
- **One domain is the whole network.** Under `egress = ["openrouter.ai"]`
  model metadata falls back to what ships in the image, and skills, plugins,
  MCP servers, web search and browsing have nothing to reach. All of it fails
  soft. The role sets `TIRITH_ENABLED = "0"` for the same reason: that
  scanner's binary is fetched from GitHub, so it could never install. Add
  domains to the role for the parts you want.
- **Behind ingress, give each agent its own hostname.** A subpath works when
  the proxy sets `X-Forwarded-Prefix`, but `dashboard.trusted_proxies` -
  which is what makes `X-Forwarded-Proto` count - is a config-file key only,
  and that file lives on the agent's volume rather than in the role.
  `HERMES_DASHBOARD_PUBLIC_URL` is the env form of `dashboard.public_url`.
- **`reef agent ssh ana-hermes` is a local shell in the VM.** For remote,
  certificate-gated terminals, see [remote access](/docs/enterprise/access).
