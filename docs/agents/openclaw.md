# OpenClaw

An [OpenClaw](https://github.com/openclaw/openclaw) 2.0 gateway in its own
microVM, on the browser image so the agent can drive a real Chromium. Nothing
to place before it boots: you pick a model provider in the browser.

`egress = ["*"]` turns off reef's deny-by-default egress, because an agent that
browses the web has to reach the web. For purpose-built agents with real egress
lists and org SSO, see [enterprise OpenClaw](/docs/enterprise/openclaw).

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
  [enterprise roles](/docs/enterprise/openclaw) when the key must stay out of
  the VM.
- **The guest verifies TLS properly.** With no `[secrets]`, microsandbox does
  not intercept, so Chromium validates real certificates. Adding a secret turns
  interception on for port 443 across the VM, and the role then needs
  `browser.extraArgs: ["--ignore-certificate-errors"]`.
- **The config is seeded once, then the agent owns it.** `[files]` writes
  `/etc/openclaw/defaults.json` into the rootfs and the `start` script copies it
  to the volume only when the copy is absent, so a role edit reaches neither
  until the VM is rebuilt. To pick one up: `reef role apply role.toml`,
  `reef agent update openclaw`,
  `reef agent exec openclaw -- rm /home/node/.openclaw/openclaw.json`, then
  `reef agent stop openclaw` and `reef agent start openclaw`.
- **Session tools reach every session on the agent.** 2026.8.2 widened the
  default from `tree` to `agent`, so the role pins `tools.sessions.visibility`
  instead of inheriting it. Set `tree` or `self` to narrow it.
- **The token is the whole boundary.** It gates the WebSocket RPC but not the
  control UI's static assets, and through the operator terminal it gets a shell
  as `node` - the access `reef agent ssh` already gives. Put the published port
  behind org ingress rather than a LAN; for certificate-gated remote terminals
  see [remote access](/docs/enterprise/access).
