# Roles

Ready-made roles: `reef role apply roles/<name>.toml`, then create agents from
them. Copy a file into your own repo to customize — pin the image to a digest,
tighten egress. Every file here is parse-checked by `cargo test`.

A volume hides whatever the image ships at its mount point, so mount the
narrowest path that holds the state you need.

- `echo` — minimal alpine role; the smallest thing that boots.
- `hermes` — the NousResearch Hermes agent with its dashboard exposed (`ui` on
  guest 9119, published to a per-agent loopback host port — see `ports` in
  `agent get`). Needs an OpenRouter key at `reef://hermes/openrouter` in
  secrets.toml, and each agent must configure a dashboard auth provider or the
  dashboard fails closed: per-agent basic auth via env (see
  [fleet/hermes.toml](../fleet/hermes.toml) — ana and bob, password
  `password`), or org SSO via `HERMES_DASHBOARD_OIDC_ISSUER` +
  `HERMES_DASHBOARD_OIDC_CLIENT_ID` in `[env]`. Serve agents on subdomains,
  not subpaths — hermes auth breaks under path prefixes upstream.
- `openclaw` — the OpenClaw gateway (`gateway` on guest 18789, published to a
  per-agent loopback host port). `init` starts it under `tini` and drops to the
  image's `node` user with `runuser`; `--bind lan` is required because
  `[expose]` only reaches services listening on `0.0.0.0`, and `[env] HOME`
  keeps state inside the volume. The whole home is the volume, which is more
  than this role writes; `openclaw-browser` below shows the narrower mount.
  Needs an OpenRouter key at `reef://openclaw/openrouter`, and each agent
  must set `OPENCLAW_GATEWAY_TOKEN` in `[env]` — auth mode `token` has no
  hashed form, so unlike the hermes password hash this value is readable
  inside the guest.
  The control UI's static assets are served without auth; the token gates the
  WebSocket RPC, so put the published port behind org ingress.
- `openclaw-browser` — the same gateway on the `-browser` image, which bakes
  Chromium into `/home/node/.cache/ms-playwright`. `egress = ["*"]` turns
  filtering off, because an agent that browses the web has to reach the web.
  The volume is `/home/node/.openclaw` alone so the mount does not hide the
  browsers, and `XDG_CACHE_HOME` moves the gateway's cache into it because
  `/home/node/.cache` is root-owned. See
  [openclaw-browser](../docs/agents/openclaw-browser.md) for the settings
  each agent needs, the extra start its first boot wants, and why Chromium
  cannot verify TLS in a guest.
