# Roles

Ready-made roles: `reef role apply roles/<name>.toml`, then create agents from
them. Copy a file into your own repo to customize — pin the image to a digest,
tighten egress. Every file here is parse-checked by `cargo test`.

A volume hides whatever the image ships at its mount point, so mount the
narrowest path that holds the state you need.

- `echo` — minimal alpine role; the smallest thing that boots.
- `hermes` — the NousResearch Hermes agent with its dashboard exposed (`ui` on
  guest 9119, published to a per-agent loopback host port — see `ports` in
  `agent get`), pinned to the digest of `v2026.8.31` (v0.21.0). `init` carries
  `gateway run`: that is what fires cron jobs, and without it the image's main
  program is the interactive CLI, which exits and stops the VM. Needs an
  OpenRouter key at `reef://hermes/openrouter` in secrets.toml, and each agent
  must configure a dashboard auth provider or the dashboard fails closed:
  per-agent basic auth via env (see [fleet/hermes.toml](../fleet/hermes.toml) —
  ana and bob, password `password`), or org SSO via
  `HERMES_DASHBOARD_OIDC_ISSUER` + `HERMES_DASHBOARD_OIDC_CLIENT_ID` in
  `[env]`. Basic auth also wants `HERMES_DASHBOARD_BASIC_AUTH_SECRET` per
  agent, or every restart logs its users out. The volume is `/opt/data` alone
  because that is the image's own `HERMES_HOME` and everything durable lives
  under it. `TIRITH_ENABLED = "0"` turns off a pre-exec scanner whose binary is
  fetched from GitHub, which this role's one-domain egress never allows.
  See [hermes](../docs/agents/hermes.md).
- `openclaw` — the OpenClaw 2.0 gateway on the browser image, pinned to the
  digest of `2026.8.1-browser` (`gateway` on guest 18789, published to a
  per-agent loopback host port). It boots with no model provider configured and
  no `[secrets]` entry: `gateway.mode = "local"` is what allows that. Connecting
  a provider at `/settings/model-setup` in the control UI is then a required
  step, because login lands in the chat and the default model has no credential. Each agent must set
  `OPENCLAW_GATEWAY_TOKEN` in `[env]` because `--bind lan` refuses to start
  without auth, and the value is readable inside the guest.
  `egress = ["*"]` turns filtering off, because an agent that browses the web
  has to reach the web. The volume is `/home/node/.openclaw` alone so the mount
  does not hide the browsers, and `XDG_CACHE_HOME` moves the gateway's cache
  into it because `/home/node/.cache` is root-owned. `[files]` ships the config
  and a `start` script that copies it into the volume on first boot, so the
  agent needs no setup after `fleet apply` and can still write its own config.
  The `${REEF_AGENT}` and `${REEF_PORT_GATEWAY}` references in that config are
  expanded by OpenClaw, not by reef: reef stores `[files]` content verbatim, so
  only reuse the pattern in roles whose app resolves env references itself.
  See [openclaw](../docs/agents/openclaw.md).
- `openclaw-marketing`, `openclaw-coding` — the same image and the same
  seeding, shaped for a team instead of a person: `gateway.auth.mode =
  "trusted-proxy"` so org SSO decides who the caller is, no gateway token
  (trusted-proxy and token auth are mutually exclusive), a narrow per-purpose
  egress list, and a separate secret ref each so provider spend separates by
  purpose. Declaring a secret turns on TLS interception for the whole VM, which
  is why these two carry `--ignore-certificate-errors` and `openclaw` does not.
  Skeletons: the domain lists and ingress hostnames are placeholders. See
  [enterprise OpenClaw](../docs/enterprise/openclaw.md) for the shape and
  [Cloudflare Access](../docs/enterprise/cloudflare-access.md) for the worked
  setup.
