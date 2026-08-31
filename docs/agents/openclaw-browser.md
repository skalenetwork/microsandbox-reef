# OpenClaw 2.0

An [OpenClaw](https://github.com/openclaw/openclaw) 2.0 gateway per person, on
the browser image so the agent can drive a real Chromium. The role configures
it, so there is nothing to set up after `fleet apply`. Copy these files into
your own repo before relying on them.

`egress = ["*"]` gives up reef's deny-by-default egress, because an agent that
browses the web has to reach the web, and `role apply` warns about it. The
secret stays bound to `openrouter.ai` and the published port stays on
loopback. Without browsing, use [openclaw](/docs/agents/openclaw) instead.

## Run

One OpenRouter key in `~/.local/state/reef/secrets.toml`, mode 0600:

```toml
[openclaw]
openrouter = "sk-or-..."
```

```sh
curl -fsSLO https://reef.clawbits.ai/roles/openclaw-browser.toml
curl -fsSLO https://reef.clawbits.ai/fleet/openclaw-browser.toml
reef role apply openclaw-browser.toml
reef fleet apply openclaw-browser.toml
```

Give each agent its own `OPENCLAW_GATEWAY_TOKEN` in the fleet file
(`openssl rand -hex 32`) first. Then open the URL `fleet apply` printed, paste
that agent's token, and approve the browser once:

```sh
reef agent exec ana-browse -- openclaw devices list
reef agent exec ana-browse -- openclaw devices approve <request-id>
```

The config also allows `http://127.0.0.1:<port>`, which skips that approval.
Prefer the named URL when several agents share a browser: cookies ignore the
port, so agents on `127.0.0.1` share one jar.

## Notes

- **The role seeds the config once; the agent then owns it.** `[files]` ships
  `/etc/openclaw/defaults.json` and a `start` script that copies it into the
  volume on first boot, leaving the agent free to write its own config -
  creating an agent or a channel does exactly that. Editing the role does not
  re-seed: delete `/home/node/.openclaw/openclaw.json` and restart for new
  defaults.
- **The volume is `/home/node/.openclaw`, not the whole home.** A reef volume
  hides what the image ships at its mount point, and this image bakes 984 MiB
  of Chromium into `/home/node/.cache/ms-playwright`. `XDG_CACHE_HOME` moves
  the gateway's cache into the volume, since `/home/node/.cache` is root-owned.
- **The guest cannot verify TLS.** reef terminates every connection host-side
  to substitute secrets, presenting a `microsandbox CA` certificate Chromium
  does not trust, so the config passes `--ignore-certificate-errors`. The
  browser cannot detect a bad certificate upstream.
- **`tools.web.search` is off deliberately.** An `OPENROUTER_API_KEY` makes
  OpenClaw treat the perplexity plugin as configured; it is not in the image,
  and the gateway will not report ready until that is resolved.
- Every other 2.0 default ships as it is, including the operator terminal: the
  agent's token gets a shell as `node`, the access `reef agent ssh` already
  gives. For remote, certificate-gated terminals see
  [remote access](/docs/enterprise/access).
- An agent that ran the earlier beta-pinned role needs a fresh volume; 2026.8.1
  will not open a per-agent database the beta wrote.
