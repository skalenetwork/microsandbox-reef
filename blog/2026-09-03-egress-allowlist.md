# What a deny-by-default egress allowlist costs

Every agent reef runs can reach the domains its role lists, and nothing else. I
wanted to write down what living with that is actually like, because the
feature-list version of it leaves out everything interesting.

## The provider is the easy part

You get that one right on the first try. It is the reason you wrote the role:

```toml
[network]
egress = ["openrouter.ai"]

[secrets]
OPENROUTER_API_KEY = { ref = "reef://hermes/openrouter", host = "openrouter.ai" }
```

Two lines, done. Then you start the agent and find out what else it wanted.

Because it is never only the provider. A package install reaches for a registry,
and the registry redirects to a CDN on some other domain. Something wants to
report a crash. Something else phones home with telemetry and either fails
quietly, which you will not notice for a week, or fails loudly and takes the
process down with it.

None of that is the sandbox being awkward. It is the first time anyone has
actually looked at what the thing talks to.

## Then it breaks in ways that are your own fault

Three that got us.

The Hermes image ships a scanner that runs before anything else, and that
scanner downloads its own binary from GitHub at startup. Our role allows one
domain. So the download hangs, and the agent hangs with it, and the logs are not
especially forthcoming about why. We set `TIRITH_ENABLED = "0"` and moved on,
which means we turned off a security tool because letting it call home was worse
than not having it. Still not sure how I feel about that one. The role is
smaller for it.

The second took longer to work out. We added a secret and OpenClaw stopped
booting. Secrets in reef never go into the guest: the role points at a `reef://`
reference bound to one host, the VM gets a placeholder, and the real value is
swapped in host-side during TLS. To do that you have to terminate TLS. Which
means the guest is now seeing an interception certificate on *every* outbound
connection, not only the one the secret is for. OpenClaw's gateway took one look
and refused to start. The fix is two lines, `--ignore-certificate-errors` and
`NODE_EXTRA_CA_CERTS`, but I had the model wrong in my head for a while. I
assumed the secret boundary was scoped to the host it was bound to. It is not.
It changes the TLS environment for everything in that VM, including code that
will never go near the secret.

The third had nothing to do with the network at all. Declaring
`OPENROUTER_API_KEY` made OpenClaw decide that its web search provider was
configured, and it then refused to start because that plugin was not bundled. So
the role also seeds `tools.web.search.enabled = false`. A credential is not
inert. It is an input to the application's own feature detection, and you find
this out by having your agent refuse to boot.

## What it does not cost

Anything at runtime, as far as we can tell. The allowlist is enforced at DNS,
outside the guest, so there is no proxy to babysit inside the VM, no CA to push
into every process, and nothing that depends on the application cooperating. An
agent that ignores `HTTPS_PROXY` completely is still in the box.

There is one consequence of that worth knowing. Since enforcement happens at
DNS, a connection to a raw IP only works while a live DNS answer for an allowed
domain is pinning that address, and only for as long as the record's TTL.
Anything that hardcodes IPs will not work. Usually that is exactly what you
want. Once in a while it is a surprise.

The host is not reachable either, and that part is not up for negotiation. An
agent cannot get to the reef host's loopback or to another agent's published
port, whatever its egress list says.

## What I would put in on day one

The model endpoint and the git remote. That is genuinely it.

Everything after that goes in when a run fails, and you add it having thought
about it for a second, because each new domain is one more place the agent's
credentials can end up. That is the part I did not expect to like. The list
stays short because a person is writing it, rather than long because a tool
watched traffic for a week and turned whatever it saw into policy.

## The one we left wide open

The OpenClaw role in our repo ships with `egress = ["*"]`. An agent whose job is
browsing has to be able to browse, and a role that could not do that would have
been useless to everyone.

You do have to write that rule out yourself, and reef mentions it every time you
apply it:

```
warn   openclaw disables egress filtering; its agents reach any host
```

That is the bit I care about. Off by default and quiet about it is the thing we
were trying to get away from. If a role gives up the boundary, the file says so
in plain sight, and the tool tells you again on the way past.
