# OpenClaw

Give each person their own [OpenClaw](https://github.com/openclaw/openclaw)
gateway: one microVM apiece, a private control UI, and nothing reachable on
the network but `openrouter.ai`.

The role and fleet files below are a reference example. Copy them into your
own repo before relying on them, and pin the image to a digest: the role
tracks `latest`.

## Secret

The role spends one OpenRouter key. Put it in
`~/.local/state/reef/secrets.toml` (mode 0600 or reef refuses to read it):

```toml
[openclaw]
openrouter = "sk-or-..."
```

The value is substituted host-side against `openrouter.ai`. The guest only
ever sees a placeholder.

OpenRouter is only what this example picks. Any provider the agent supports
works: rename the key in `[secrets]`, point its `host` at that provider's API,
and put the same domain in the role's `[network] egress`.

## Fleet

One entry per person. `owner` is who may open a terminal into the agent
later; the token is the gateway's bearer credential, which every agent must
set because OpenClaw refuses to bind beyond loopback without auth:

```toml
version = 1

[agents.ana-openclaw]
role = "openclaw"
owner = "ana"
env = { OPENCLAW_GATEWAY_TOKEN = "change-me-ana" }
```

Give each agent its own value - `openssl rand -hex 32` - and treat it as a
shared secret, not an identity: it says nothing about who is connecting.

## Run

```sh
curl -fsSLO https://reef.clawbits.ai/roles/openclaw.toml
curl -fsSLO https://reef.clawbits.ai/fleet/openclaw.toml
reef role apply openclaw.toml
reef fleet apply openclaw.toml
```

`fleet apply` prints each agent's URL as it creates it, and `agent get` shows
it again later. Replace the shipped `change-me-*` tokens before this reaches
anyone.

Each agent then needs two settings written once. OpenClaw ships a default
model that is not OpenRouter, so it would ignore the key and fail on its first
reply; and the control UI trusts only origins it is told about plus its own
loopback hostnames, so an `<agent>.localhost` URL is rejected with `origin
not allowed` until you name it:

```sh
reef agent exec ana-openclaw -- openclaw config set \
  agents.defaults.model.primary openrouter/auto
reef agent get ana-openclaw
reef agent exec ana-openclaw -- openclaw config set \
  gateway.controlUi.allowedOrigins \
  '["http://ana-openclaw.localhost:19000"]' --strict-json
reef agent stop ana-openclaw && reef agent start ana-openclaw
```

Take the port from `agent get`. reef allocates the lowest free one from
19000 upwards, so every agent needs its own entry. Both settings land in
`openclaw.json` inside the volume, so they survive restarts and recreates.
The gateway may exit while these run, so the restart is required rather than
tidy.

Then open that URL and paste the agent's token into the control UI. The
token alone is not enough: OpenClaw asks the browser to pair, and the page
prints the request id to approve.

```sh
reef agent exec ana-openclaw -- openclaw devices list
reef agent exec ana-openclaw -- openclaw devices approve <request-id>
```

`openclaw models status` inside an agent confirms the wiring: the default
reads `openrouter/auto`, and openrouter's key shows as an `MSB_` placeholder,
which is the real value being substituted host-side and never entering the
guest.

Re-run `reef fleet apply` after editing the file. An env change restarts the
agent in place; only a role change recreates the VM.

## Notes

- The whole of `/home/node` is the volume, though `.openclaw` is the only
  path this role writes: it holds the config, SQLite databases and
  workspace. [openclaw-browser](/docs/agents/openclaw-browser) mounts that
  path alone; changing this role to match would strand existing agents' data.
- The token gates the WebSocket RPC, chat and `/v1/*` routes. The control
  UI's static assets and health probes are served unauthenticated, so put
  the published port behind the org's ingress rather than on a LAN.
- Behind an authenticating proxy, OpenClaw can defer to it with auth mode
  `trusted-proxy`, which needs the proxy's address in `gateway.trustedProxies`
  and fails closed without it.
- `reef agent ssh ana-openclaw` is a local shell in the VM. For remote,
  certificate-gated terminals, see [remote access](/docs/enterprise/access).
