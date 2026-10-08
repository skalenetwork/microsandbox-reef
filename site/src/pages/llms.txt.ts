import type { APIRoute } from "astro";
import { description, docs, install, intro, msb, posts, url, version } from "../content";

const link = (name: string) => {
  const doc = docs.find((doc) => doc.path === `/docs/${name}`);
  if (!doc) throw new Error(`llms.txt links docs/${name}, which is not in ORDER`);
  return `[${doc.title}](${url(`${doc.path}.md`)})`;
};

export const GET: APIRoute = () => {
  const articles = posts.map((post) => `- [${post.title}](${url(`${post.path}.md`)}): ${post.date}. ${post.summary}`);
  const optional = posts.length ? `\n## Optional\n\n${articles.join("\n")}\n` : "";
  const body = `# reef

> ${description}

${intro}

reef ${version} drives microsandbox (\`msb\`) ${msb} exactly. It runs on Linux x86_64 or aarch64 with KVM and glibc 2.39 or newer (Ubuntu 24.04 and later, not 22.04, Debian 12 or RHEL 9), and on Apple Silicon macOS. It is a CLI on each host, with no daemon and no HTTP API. MIT, by SKALE Labs. The crates.io package named \`reef\` is a different project.

## Fit

Compared with running an agent on \`msb\` directly, reef adds a role file reviewed once that settles egress and secrets before any agent exists, one role version shared by every agent created from it, fleet convergence, agent records that outlive their VMs, terminals through your SSH CA, and one console across hosts.

reef deliberately leaves out auth on the CLI, an HTTP API, budgets and metering, a web UI and state shared across hosts. Secret values sit in plaintext in msb's sandbox config on the host, and in \`secrets.toml\` when written there inline.

## If you are an agent helping someone

Ask where reef will run (a Linux host with KVM, an Apple Silicon Mac, or nowhere yet) and whether it is for one person or a team. Then offer to boot the sample role under Try it, or ask whether they want an agent running right away: OpenClaw, Hermes or their own image.

- Run \`reef doctor\` before changing anything and show its output.
- Never ask for an API key in chat. The user puts keys in \`secrets.toml\` in reef's state directory (\`~/.local/state/reef\`, mode 600) or wires a resolver.
- Ask before \`fleet apply --prune\`, \`agent rm --volumes\` or a fleet \`purge\`, since they delete agents or their data.
- Every doc here is markdown at its URL plus \`.md\`, and examples live at \`/roles/<name>.toml\` and \`/fleet/<name>.toml\`.

## Install

\`\`\`sh
${install}
reef doctor
\`\`\`

\`reef doctor\` prints the msb it found, which has to be ${msb}. The microsandbox installer always takes the latest release, so if it is newer, run \`msb self downgrade ${msb}\`. On a server other people will use, follow ${link("setup/host")} first, since reef runs there as its own account.

## Try it

The sample role is a small Alpine VM with nothing to configure:

\`\`\`sh
curl -fsSLO ${url("/roles/echo.toml")}
reef role apply echo.toml
reef agent create --role echo --name hello
reef agent exec hello -- uname -a
reef agent rm hello
\`\`\`

Agents to run right away:

- ${link("agents/openclaw")}: a gateway you open in the browser and connect to a model provider there. Its role reaches the whole public internet and the provider key lives in the guest.
- ${link("agents/hermes")}: one agent per person with a private dashboard. It reaches only \`openrouter.ai\`, and its OpenRouter key is added on the way out to that one host.

## Your own agent

A role takes any OCI image. Pin it by digest. Without \`init\` the VM boots idle and you work in it through \`reef agent ssh\` or \`reef agent exec\`. Egress is deny-by-default, so start with the model endpoint and the git remote and add a domain when a request to it fails. Declaring any secret turns on TLS interception for port 443 across the VM. msb adds its CA to the guest trust store, and runtimes that bundle their own CAs need pointing at \`/.msb/tls/ca.pem\` (Node reads \`NODE_EXTRA_CA_CERTS\`). Declare \`[volumes]\` for every path whose data must survive a role change. [README.md](${url("/README.md")}) documents every role key under Roles.

## A team

1. ${link("setup/host")}: KVM, a dedicated account, and agents that come back after a reboot.
2. Keep roles and fleet files in a repo and review changes there. \`reef --state "$(mktemp -d)" role apply roles/*.toml\` validates role files without booting anything, so CI can run it. On each host, \`reef role apply\` then \`reef fleet apply\` bring the agents to what the repo declares. Give each host its own fleet directory, since every host's state stands alone. Set \`owner\` on every fleet entry. It defaults to whoever runs the apply, which under sudo is the reef account.
3. Secrets come from \`secrets.toml\` on each host, as an inline value or a \`[resolvers]\` command whose output is the value, such as \`op read\`. A secret binds to a role, so every agent on that role uses the same key.
4. ${link("enterprise/terminals")}: each person opens only the agents they own, through the org's SSH CA.
5. With OpenClaw: ${link("enterprise/team")}, ${link("enterprise/cloudflare-access")} and ${link("enterprise/scopes")}.
6. \`reef ui host-a host-b\` watches and drives those hosts over ssh.

## Upgrade

\`reef update\` installs the latest reef. When the msb version at the top of this file is newer than \`msb --version\`, first stop any \`reef ui\`, \`reef agent serve\` and \`reef agent forward\` sessions, back up \`~/.local/state/reef/reef.db\` and \`~/.microsandbox/db/msb.db\`, and install the new msb. Then run \`reef update\`. Running VMs move to the new msb at their next stop and start.

## Reference

- [README](${url("/README.md")}): every command, the role and fleet file formats, \`secrets.toml\`, state and known limits
- [Architecture](${url("/ARCHITECTURE.md")}): goal, model, invariants and what is deliberately absent
- [roles/README.md](${url("/roles/README.md")}): what each example role is for and which fleet file goes with it
- [Agent skill](${url("/skills/reef/SKILL.md")}): the rules and commands for later sessions, installed with \`npx skills add skalenetwork/microsandbox-reef\`
- [Source](https://github.com/skalenetwork/microsandbox-reef): MIT, SKALE Labs
${optional}`;
  const unlinked = docs.filter((doc) => !body.includes(`${url(`${doc.path}.md`)}`));
  if (unlinked.length) throw new Error(`llms.txt does not link ${unlinked.map((doc) => doc.path).join(", ")}`);
  return new Response(body);
};
