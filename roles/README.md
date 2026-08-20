# Roles

Ready-made roles: `reef role apply roles/<name>.toml`, then create agents from
them. Copy a file into your own repo to customize — pin the image to a digest,
tighten egress. Every file here is parse-checked by `cargo test`.

- `echo` — minimal alpine role; the smallest thing that boots.
- `hermes` — the NousResearch Hermes agent, dashboard served on guest loopback:
  `reef agent forward <agent> 9119`, then open http://127.0.0.1:9119. Needs an
  OpenRouter key at `reef://hermes/openrouter` in secrets.toml. For standing
  ingress instead of `forward`, set `HERMES_DASHBOARD_HOST = "0.0.0.0"`, add an
  `[expose]` entry, and configure a hermes dashboard auth provider.
