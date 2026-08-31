# OpenClaw with a browser

Give each person an [OpenClaw](https://github.com/openclaw/openclaw) gateway
that can drive a real browser: one microVM apiece, a private control UI, and
Chromium baked into the image.

The role and fleet files below are a reference example. Copy them into your
own repo before relying on them. The image is pinned to a digest, and that
digest is a beta: no stable 2026.8 or 2026.9 release exists yet.

An agent that browses the web has to reach the web, so this role sets
`[network] egress = ["*"]` and gives up reef's deny-by-default egress.
`reef role apply` warns when it reads such a role. The rest of the boundary
holds: the secret stays bound to one host, and the published port stays on
loopback. If you do not need browsing, use the
[openclaw](/docs/agents/openclaw) role and keep the egress list.

## Secret

The role spends one OpenRouter key, the same one the `openclaw` role uses.
Put it in `~/.local/state/reef/secrets.toml` (mode 0600 or reef refuses to
read it):

```toml
[openclaw]
openrouter = "sk-or-..."
```

The value is substituted host-side against `openrouter.ai` and never enters
the guest, even though egress is open: the binding is a property of the
secret, not of the egress list.

## Fleet

One entry per person. `owner` is who may open a terminal into the agent
later; the token is the gateway's bearer credential, which every agent must
set because OpenClaw refuses to bind beyond loopback without auth:

```toml
version = 1

[agents.ana-browse]
role = "openclaw-browser"
owner = "ana"
env = { OPENCLAW_GATEWAY_TOKEN = "change-me-ana" }
```

Give each agent its own value (`openssl rand -hex 32`) and treat it as a
shared secret, not an identity.

## Run

```sh
curl -fsSLO https://reef.clawbits.ai/roles/openclaw-browser.toml
curl -fsSLO https://reef.clawbits.ai/fleet/openclaw-browser.toml
reef role apply openclaw-browser.toml
reef fleet apply openclaw-browser.toml
```

The first boot ends with the VM stopped, which is expected. OpenClaw's
startup doctor installs a missing plugin from npm, which open egress lets it
do, then refuses to report ready and exits so migrations run against the
final plugin inventory. Start the agent again and it stays up:

```sh
reef agent start ana-browse
```

Each agent then needs four settings written once. None of them has a CLI flag
or an environment variable, so all four are `config set` calls. The last one
names that agent's own URL: take the port from `agent get`, because reef
allocates the lowest free one from 19000 upwards and no two agents share it.

```sh
reef agent get ana-browse
reef agent exec ana-browse -- openclaw config set \
  agents.defaults.model.primary openrouter/auto
reef agent exec ana-browse -- openclaw config set \
  browser.extraArgs '["--ignore-certificate-errors"]' --strict-json
reef agent exec ana-browse -- openclaw config set \
  gateway.terminal.enabled false --strict-json
reef agent exec ana-browse -- openclaw config set \
  gateway.controlUi.allowedOrigins '["http://ana-browse.localhost:19000"]' \
  --strict-json
reef agent stop ana-browse && reef agent start ana-browse
```

OpenClaw ships `openai/gpt-5.6-sol`, which would ignore the OpenRouter key;
`browser.extraArgs` is what lets Chromium load a page at all (see Notes); and
`gateway.terminal.enabled` closes an operator shell that is now on by
default. The gateway exits while these run, so the restart is required rather
than tidy.

Then open the agent's URL from `agent get` and paste the agent's token. The
token alone is no longer enough: OpenClaw asks the browser to pair, and the
page prints the request id to approve.

```sh
reef agent exec ana-browse -- openclaw devices list
reef agent exec ana-browse -- openclaw devices approve <request-id>
```

Reconnect after approving. `openclaw browser open https://example.com`
inside the agent confirms the whole path works and prints the page title.

Re-run `reef fleet apply` after editing the file. An env change restarts the
agent in place; only a role change recreates the VM.

## Notes

- The volume is `/home/node/.openclaw`, not the whole home. A reef volume
  hides whatever the image has at its mount point, and this image bakes
  984 MiB of Chromium into `/home/node/.cache/ms-playwright`. Mounting over
  `/home/node` would hide it, and nothing downloads it back: the runtime
  cannot install browsers, so the browser tool fails with `No supported
  browser found`.
- `XDG_CACHE_HOME` is set into the volume because `/home/node/.cache` is
  root-owned in the image and the gateway runs as `node`. Without it,
  startup migrations fail and the gateway never reports ready.
- The guest cannot verify TLS. reef terminates every connection host-side to
  substitute secrets, presenting a `microsandbox CA` certificate. Node trusts
  it through `NODE_EXTRA_CA_CERTS`; Chromium keeps its own trust store and
  does not, so without `--ignore-certificate-errors` every page is a
  certificate interstitial. The flag makes the browser trust an interception
  the rest of the VM already lives under, at the cost of not noticing a bad
  certificate upstream.
- `browser.noSandbox` is not needed: the guest allows unprivileged user
  namespaces, so Chromium's own sandbox starts.
- The browser's control service and its CDP ports bind loopback inside the
  guest. Only `gateway` is published. Do not expose the others.
- The token gates the WebSocket RPC. Several surfaces are not token-gated,
  including the control UI's static assets and the `/healthz`, `/readyz` and
  `/startupz` probes, so put the published port behind the org's ingress
  rather than on a LAN.
- A paired device holds `operator.admin`, which is why the run steps turn
  the operator terminal off. Leaving it on gives anyone with the token a
  shell in the VM as `node`.
- The idle gateway holds about 574 MiB, and one Chromium with a single page
  adds roughly 1 GiB across 14 processes. `max-pids` is left unset because
  Chromium's process tree hits it as `EAGAIN` rather than a clear error.
- `reef agent ssh ana-browse` is a local shell in the VM. For remote,
  certificate-gated terminals, see [remote access](/docs/enterprise/access).
