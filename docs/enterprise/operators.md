# Who can do what

The Access policy on an agent's hostname decides who gets in. What they can do
once inside is OpenClaw's, and it is three settings that compose rather than one.
This page is what the [enterprise roles](/docs/enterprise/openclaw) ship, and how
to give one named person more.

```text
Give this to your agent:

Read https://reef.clawbits.ai/docs/enterprise/operators.md before changing any
scope on a running agent. Tell me which of the three settings you are changing
and why. Never put operator.admin in deviceAutoApprove. Write config as the node
user, never as root.
```

## The three settings

| setting | grants to | lives in |
| --- | --- | --- |
| `gateway.auth.trustedProxy.deviceAutoApprove.scopes` | every browser that passes SSO | the role, seeded once |
| `gateway.auth.identityScopes` | one named email | config, editable live |
| `gateway.roles` | a ceiling every session is filtered through | config plus runtime state |

They are not alternatives. The device grant is capped by `deviceAutoApprove` when
a browser first pairs, `identityScopes` is unioned on top at every Control UI
connection, and the `gateway.roles` ceiling, if configured, filters the result
last.

## What the roles ship

```json
"deviceAutoApprove": {
  "enabled": true,
  "scopes": ["operator.read", "operator.write", "operator.approvals", "operator.questions"]
}
```

That is the shared baseline, and it is byte-identical to OpenClaw's own default;
the roles state it explicitly so a reviewer can read it. `enabled: true` is a
requirement rather than a convenience: a trusted-proxy Control UI session with no
paired device is admitted and then stripped of every scope, and it cannot approve
its own pairing.

`operator.admin` is deliberately absent. It is the one scope that
short-circuits every check, and it alone unlocks the operator terminal, config
mutation, the secret store, and session deletion. In `deviceAutoApprove` it would
go to every browser that passed SSO rather than to a person.

Note that `deviceAutoApprove.scopes` is validated as plain strings, not against
the scope list, so a misspelled scope boots cleanly and silently narrows what
your team can do.

## Granting one person admin

`identityScopes` is the only per-person grant that lives in config. It is keyed
by the email the proxy supplied, matched exactly and then case-insensitively for
keys containing `@`, and it is unioned with the device's scopes on every
connection:

```sh
reef agent exec coding -- runuser -u node -- env HOME=/home/node \
  openclaw config set gateway.auth.identityScopes \
  '{"ana@example.com":["operator.admin"]}' --strict-json
```

Use the whole-map form. An email contains dots, so a dotted path would be parsed
as path segments. `runuser -u node` is not optional: the config is node-owned
`0600`, and a write as root replaces it by rename and leaves it root-owned, which
breaks the gateway's own config writes from then on. The command reports whether
a restart is needed; `reef agent stop` then `reef agent start` keeps the volume,
so the config, the sessions and the user profiles all survive.

The grant applies on the person's next connection, so they reload the tab. The
gateway logs `identity scope grant elevated connection` when it happens.

**The trap.** When the Control UI says *Administrator access required*, it
suggests running `openclaw devices` on the gateway. That cannot work here. The
CLI is a gateway client, and under `trusted-proxy` a loopback peer is refused
before any pairing logic runs, so an in-VM invocation returns a bare
`unauthorized`. Use `identityScopes`.

## Why the roles do not ship gateway.roles

`gateway.roles` looks like the answer to per-person permissions and is not. It
carries named capability bundles and a default, with **no email, group or pattern
matching of any kind**. Binding a person to a bundle is durable state inside the
agent, written only by the `users.setRole` RPC, which requires `operator.admin`
and has neither a CLI nor a Control UI.

Configuring it also changes failure from soft to hard: an identity with no
resolvable profile gets an empty scope set rather than the default, and the
device handshake is refused outright. A role file cannot seed the assignments
that would make that safe, so the roles leave it unset and everyone shares the
baseline until you name them individually.

## What is still shared

Scopes decide what a session may call, not what it can see. On one agent there is
one session list, one workspace, one credential pool and one browser cookie jar,
whatever anyone's scopes are. When people should not share those, give them
separate agents rather than separate scopes. See
[enterprise OpenClaw](/docs/enterprise/openclaw).

## The other axis

Scopes govern the browser. A terminal inside the VM is
[remote access](/docs/enterprise/access): an SSH certificate whose principal
matches the agent's `owner`, which is a different identity, a different audit
trail, and unaffected by anything on this page.
