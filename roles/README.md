# Roles

Ready-made roles: `reef role apply roles/<name>.toml`, then create agents from
them. Copy a file into your own repo to customize — pin the image to a digest,
tighten egress. Every file here is parse-checked by `cargo test`.

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
