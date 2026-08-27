# Hermes

Give each person their own [Hermes](https://github.com/NousResearch/hermes-agent)
agent: one microVM apiece, a private dashboard, and nothing reachable on the
network but `openrouter.ai`.

The role and fleet files below are a reference example. Copy them into your
own repo before relying on them, and pin the image to a digest: the role
tracks `latest`.

## Secret

The role spends one OpenRouter key. Put it in
`~/.local/state/reef/secrets.toml` (mode 0600 or reef refuses to read it):

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
later; the two env keys are the dashboard's basic auth, which every agent
must set or the dashboard fails closed:

```toml
version = 1

[agents.ana-hermes]
role = "hermes"
owner = "ana"
env = { HERMES_DASHBOARD_BASIC_AUTH_USERNAME = "ana", HERMES_DASHBOARD_BASIC_AUTH_PASSWORD_HASH = "scrypt$16384$8$1$..." }
```

For org SSO instead, set `HERMES_DASHBOARD_OIDC_ISSUER` and
`HERMES_DASHBOARD_OIDC_CLIENT_ID` in the role's `[env]` and leave the basic
auth keys out.

## Run

```sh
curl -fsSLO https://reef.clawbits.ai/roles/hermes.toml
curl -fsSLO https://reef.clawbits.ai/fleet/hermes.toml
reef role apply hermes.toml
reef fleet apply hermes.toml
reef agent get ana-hermes
```

The `ports` line names the loopback host port carrying guest 9119. Open it
and log in as `ana`. The shipped fleet file is a demo: its hashes are the
password `password`, so replace them before this reaches anyone.

Re-run `reef fleet apply` after editing the file. An env change restarts the
agent in place; only a role change recreates the VM.

## Notes

- `/opt/data` is the only path that outlives a recreate. Everything else is
  rootfs and is replaced by the next image.
- Behind ingress, give each agent a subdomain rather than a path prefix:
  hermes auth breaks under path prefixes upstream.
- `reef agent ssh ana-hermes` is a local shell in the VM. For remote,
  certificate-gated terminals, see [remote access](/docs/enterprise/access).
