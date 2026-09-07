# OpenClaw

An [OpenClaw](https://github.com/openclaw/openclaw) 2.0 gateway in its own
microVM, on the browser image so the agent can drive a real Chromium. Nothing
to place before it boots: you pick a model provider in the browser.

`egress = ["*"]` turns off reef's deny-by-default egress, because an agent that
browses the web has to reach the web. For purpose-built agents with real egress
lists and org SSO, see [set up a team](/docs/enterprise/team).

This page assumes a [prepared host](/docs/setup/host): reef and msb installed,
KVM working.

## Run

```sh
curl -fsSL https://reef.clawbits.ai/roles/openclaw.toml -o role.toml
curl -fsSL https://reef.clawbits.ai/fleet/openclaw.toml -o fleet.toml
```

Set your own `OPENCLAW_GATEWAY_TOKEN` in `fleet.toml` (`openssl rand -hex 32`).
`--bind lan` will not start without one, and it is a shared secret, not an
identity.

```sh
reef role apply role.toml
reef fleet apply fleet.toml
```

Open the URL `fleet apply` printed, paste the token, and approve the browser:

```sh
reef agent exec openclaw -- openclaw devices list
reef agent exec openclaw -- openclaw devices approve <request-id>
```

Then go to `/settings/model-setup` and connect a provider. Sign-in lands you in
the chat rather than a setup screen, and the default model is one you have no
credential for, so the first message fails until you do this.

## Notes

- **The provider key lives inside the guest.** reef's "spend it but never read
  it" guarantee applies to `[secrets]`, which this role does not use. Use the
  [enterprise roles](/docs/enterprise/team) when the key must stay out of
  the VM.
- **It boots with no provider because `gateway.mode` is `"local"`.** Any other
  mode wants a configured model at startup, which is why this role can ship
  without a `[secrets]` entry and hand the choice to the browser.
- **The guest verifies TLS properly.** With no `[secrets]`, microsandbox does
  not intercept, so Chromium validates real certificates. Declaring any secret
  turns interception on for port 443 across the whole VM, and the role then
  needs both `browser.extraArgs: ["--ignore-certificate-errors"]` for Chromium
  and `NODE_EXTRA_CA_CERTS = "/.msb/tls/ca.pem"` for the gateway's own outbound
  TLS, which otherwise rejects the interception certificate and refuses to
  start. The [enterprise roles](/docs/enterprise/team) carry both; this one
  carries neither.
- **The volume is `/home/node/.openclaw` alone**, so the mount does not hide the
  browsers the image ships beside it. `XDG_CACHE_HOME` moves the gateway's cache
  into that volume, because `/home/node/.cache` is root-owned.
- **The config is seeded once, then the agent owns it.** `[files]` writes
  `/etc/openclaw/defaults.json` into the rootfs and the `start` script copies it
  to the volume only when the copy is absent, so a role edit reaches neither
  until the VM is rebuilt. To pick one up: `reef role apply role.toml`,
  `reef agent update openclaw`,
  `reef agent exec openclaw -- rm /home/node/.openclaw/openclaw.json`, then
  `reef agent stop openclaw` and `reef agent start openclaw`.
- **An image bump migrates the state volume.** The volume outlives the image, so
  a new digest boots against the old `/home/node/.openclaw`. On 2026.8.2 an
  advisory migration warning exited startup, which stops the VM and is impossible
  to miss. 2026.9.1 starts degraded instead and logs one aggregate warning naming
  the Doctor check to run, so an agent that is up no longer proves the migration
  finished.
- **Session tools reach every session on the agent.** 2026.8.2 widened the
  default for unsandboxed sessions from `tree` to `agent`, and reef leaves
  OpenClaw's own sandbox off, so the role pins `tools.sessions.visibility`
  instead of inheriting it. Set `tree` or `self` to narrow it.
- **The token is the whole boundary.** It gates the WebSocket RPC but not the
  control UI's static assets, and through the operator terminal it gets a shell
  as `node` - the access `reef agent ssh` already gives. Put the published port
  behind org ingress rather than a LAN; for certificate-gated remote terminals
  see [terminal access](/docs/enterprise/terminals).

Next: [Hermes](/docs/agents/hermes), one agent per person with a real egress
list and a key the VM never reads.
