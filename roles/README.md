# Roles

Ready-made roles: `reef role apply roles/<name>.toml`, then create agents from
them. Copy a file into your own repo to customize: pin the image to a digest,
tighten egress. Every role here is parse-checked by `cargo test`.

- `echo`: minimal alpine role, the smallest thing that boots.
- `hermes`: the NousResearch Hermes agent with its dashboard exposed, one domain
  of egress, and an OpenRouter key it spends but never reads. Each agent has to
  configure a dashboard auth provider or the dashboard fails closed. See
  [hermes](../docs/agents/hermes.md) and
  [fleet/hermes.toml](../fleet/hermes.toml).
- `openclaw`: an OpenClaw 2.0 gateway on the browser image. `egress = ["*"]`, no
  secrets, and no provider configured: you pick one in the browser. Each agent
  sets its own `OPENCLAW_GATEWAY_TOKEN`, readable inside the guest. See
  [openclaw](../docs/agents/openclaw.md) and
  [fleet/openclaw.toml](../fleet/openclaw.toml).
- `openclaw-marketing`, `openclaw-coding`: the same image and seeding shaped for
  a team behind org SSO, with a narrow per-purpose egress list and a separate
  provider key each, so spend separates by purpose. Skeletons: the domain lists
  are placeholders. See [set up a team](../docs/enterprise/team.md) for the
  shape and why each field is set,
  [browser access](../docs/enterprise/cloudflare-access.md) for the worked
  setup, and [fleet/openclaw-team.toml](../fleet/openclaw-team.toml).
- `clawbits-openclaw`: the same gateway on the [Clawbits](https://clawbits.ai)
  image, which bakes the clawbits plugins in and boots through its own entrypoint.
  `[volumes]` mounts `state` and `workspace` separately and never
  `/home/node/.openclaw`: a volume at the parent hides those plugins and the
  agent degrades to stock OpenClaw. `CLAWBITS_ENDPOINT` in `[env]` is the
  deployment every agent on this role enrols into — Clawbits offers the role only
  to the org whose server it names, so a copy pointed elsewhere is a different
  role. Each agent gets `CLAWBITS_ORG_ID` and a one-time `CLAWBITS_SIGNUP_TOKEN`
  from its fleet file ([fleet/clawbits.toml](../fleet/clawbits.toml)); it spends
  the token at first boot and keeps its own key on the `state` volume, so the
  file is dead weight afterwards and a recreate comes back as the same agent. The
  gateway token is a property of the machine, minted into the state volume on
  first boot, so it never travels through the fleet file:
  `reef agent exec NAME -- cat /home/node/.openclaw/state/gateway-token` reads it.
  Left without an org and token, the agent runs detached, which is a valid mode.
