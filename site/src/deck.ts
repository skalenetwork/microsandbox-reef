export const problem = [
  { title: "They run code and browse", text: "Agents run commands and open websites with no person approving each step." },
  { title: "They hold credentials", text: "A key an agent can read is a key it can print, log or send somewhere else." },
  { title: "Their settings are not a boundary", text: "Anything reaching the agent, including its own input, can talk it out of them." },
];

export const questions = [
  { q: "What can it reach?", a: "Only the domains in its role. Opening the whole internet is an explicit setting." },
  { q: "Which credentials can it use?", a: "Keys named in the role stay on the host and work toward one service." },
  { q: "Who approved its setup?", a: "Roles are files in git. Protect the branch and every change is reviewed." },
  { q: "Who can use it?", a: "A sign-on proxy decides who reaches it. A terminal needs a certificate from your SSH CA." },
];

export const parts = [
  { title: "A role", text: "One file per agent type: image, resources, the domains it may reach, the keys it may use." },
  { title: "A microVM", text: "Every agent gets its own kernel, its own disk and a volume that outlives it." },
  { title: "One binary", text: "No daemon. A command reconciles the VM to the record, then exits." },
  { title: "Your servers", text: "Linux with KVM or an Apple Silicon Mac. MIT licensed, nothing phones home." },
];

export const steps = [
  { n: "1", title: "Define", text: "The platform team writes a role and merges it." },
  { n: "2", title: "Launch", text: "Teams create agents from approved roles, no ticket." },
  { n: "3", title: "Run", text: "Each agent runs in its own microVM, held to its role." },
];

export const role = [
  { k: "Software", v: "OpenClaw, pinned to an exact image digest" },
  { k: "Resources", v: "4 vCPU · 8 GB memory · 20 GB disk" },
  { k: "Network", v: "openrouter.ai · github.com · api.github.com · registry.npmjs.org" },
  { k: "Credentials", v: "An OpenRouter key, usable only toward openrouter.ai" },
  { k: "Storage", v: "10 GB that survives restarts and upgrades" },
];

export const notes = [
  { title: "Anything not listed is blocked", text: "Checked outside the agent, so it holds even when the agent ignores its own settings." },
  { title: "Keys are references", text: "The agent holds a placeholder. The real value is added on the way out." },
  { title: "Versions are tracked", text: "A new role version flags every agent still running the old one." },
];

export const cases = [
  { title: "Engineering agent", text: "Works in code repositories and installs packages.", tags: ["model provider", "GitHub", "npm"] },
  { title: "Marketing agent", text: "Works in the marketing stack and nowhere else.", tags: ["model provider", "HubSpot", "LinkedIn"] },
  { title: "One agent per employee", text: "A private Hermes agent per person, each in its own microVM.", tags: ["model provider"] },
  { title: "Agents in team chat", text: "Launched from Clawbits, joining your channels as a member.", tags: ["public internet"] },
];

export const stack = [
  { k: "Identity", v: "Cloudflare Access, or any sign-on proxy such as oauth2-proxy or Tailscale" },
  { k: "Change control", v: "Git and pull request review, with GitHub for the Clawbits integration" },
  { k: "Secrets", v: "1Password, OpenBao or AWS Secrets Manager, read when the VM is created" },
  { k: "Terminals", v: "Your existing SSH certificate authority" },
  { k: "Agents", v: "OpenClaw and Hermes, with roles that ship in the repo" },
  { k: "Servers", v: "Linux with KVM and glibc 2.39 or newer, x86_64 or ARM, and Apple Silicon Macs" },
];

export const comparison = {
  columns: ["Containers", "Traditional VMs", "microsandbox"],
  rows: [
    { k: "Own kernel per agent", v: ["Shared host kernel", "Yes", "Yes"] },
    { k: "Runs standard container images", v: ["Yes", "Not directly", "Yes"] },
    { k: "Blocks unapproved domains outside the agent", v: ["Needs extra tooling", "Needs extra tooling", "Built in"] },
    { k: "Keeps API keys out of the agent", v: ["Needs extra tooling", "Needs extra tooling", "Built in"] },
  ],
};

export const benefits = [
  { title: "Containment", text: "Each agent is isolated in its own microVM and reaches only approved services." },
  { title: "Governance", text: "Roles are reviewable files, and every agent maps to a role version." },
  { title: "Self-service", text: "Teams launch agents from approved roles without waiting on tickets." },
  { title: "Ownership", text: "Agents run on your servers. reef is open source under the MIT license." },
];

export const invariants = [
  { title: "No daemon", text: "Every mutating command runs one reconcile pass, then returns." },
  { title: "Deterministic planning", text: "A pure function turns 18 cases into five actions: create, modify, start, stop, remove." },
  { title: "One runtime dependency", text: "A single module names a microsandbox type, pinned to an exact release." },
  { title: "Durable records", text: "Agent records outlive their VMs. A VM is rebuilt from its record." },
];

export const controls = [
  { title: "At DNS, by the runtime", text: "The allowlist is checked outside the guest. A name that is not listed never resolves." },
  { title: "At the egress proxy", text: "A secret is bound to one domain. The placeholder is swapped for the value on the way out." },
  { title: "At the host loopback", text: "Agent ports bind to 127.0.0.1, which no agent can reach, whatever its role allows." },
];

const kept = (label: string) => ({ label, kept: label === "Kept" });

export const changes = [
  { k: "Stop and start", v: "Stops and starts the VM", cells: ["Kept", "Kept", "Kept"].map(kept) },
  { k: "Environment change", v: "Restarts the VM in place", cells: ["Kept", "Kept", "Kept"].map(kept) },
  { k: "New role version", v: "Replaces the VM", cells: ["Replaced", "Kept", "Kept"].map(kept) },
  { k: "Remove agent", v: "Removes the VM", cells: ["Removed", "Kept", "Released"].map(kept) },
  { k: "Remove with volumes", v: "Removes the VM and its volumes", cells: ["Removed", "Deleted", "Released"].map(kept) },
];

export const request = [
  "The browser opens the agent's hostname, which resolves to Cloudflare.",
  "Access signs the person in with your identity provider, and nothing else.",
  "cloudflared checks the signed token on the tunnel it dialled out from the host.",
  "It forwards to 127.0.0.1, and the agent opens a session under that email.",
];

export const reach = [
  { k: "Clawbits", v: "One private repository, through a token you scope to it" },
  { k: "reef host", v: "The same repository, through its own deploy key" },
  { k: "Agent", v: "Outbound only, to the domains its role allows" },
  { k: "Inbound", v: "Nothing connects in to the host" },
];

export const status = [
  {
    title: "Available today",
    items: [
      "Open source under the MIT license",
      "Linux with KVM and glibc 2.39 or newer, and Apple Silicon Macs",
      "Roles for OpenClaw and Hermes that ship in the repo",
      "The Clawbits integration, over git",
    ],
  },
  {
    title: "Added in 0.15",
    items: [
      "Terminals through the microsandbox SSH server, no per-person enrollment",
      "reef reconcile brings agents back after a reboot or a crash",
      "reef secret rotate pushes a changed key into running agents",
      "reef migrate moves a 0.14 host onto the current runtime, volumes kept",
    ],
  },
  {
    title: "Known limits",
    items: [
      "Secret values sit unencrypted on the host, in two places; reef doctor warns",
      "No HTTP API yet, and it will not ship without auth",
      "A role upgrade replaces the system disk; volumes carry over",
      "microsandbox is pre-1.0, pinned exactly, with no support for older releases",
    ],
  },
];

export const clawbits = [
  { title: "Any member can launch one", text: "They pick a role and a machine in the app. No ticket, and no access to the server." },
  { title: "On the organization's machines", text: "Agents run on servers the customer owns, not on Clawbits infrastructure." },
  { title: "Clawbits never connects in", text: "One private git repository is the whole channel between the two systems." },
];

export const integration = [
  { k: "Launch an agent without touching the server", v: "One fleet file per agent, committed to a branch every host pulls" },
  { k: "One definition for every agent in the org", v: "The clawbits-openclaw role, reviewed once and pinned to an exact image" },
  { k: "An agent that survives a rebuild", v: "Named volumes: it keeps its own key and comes back as the same agent" },
  { k: "A terminal only for the person who owns it", v: "owner in the fleet file, checked by agent serve against the certificate" },
  { k: "No way in to a customer machine", v: "The host pulls and pushes through its own deploy key; nothing connects in" },
];

export const terminalConsole = [
  { title: "Every host at once", text: "One console merges agents from independent hosts, reached over your own ssh." },
  { title: "Full detail", text: "Role version, image, owner, resources, volumes, egress policy and ports." },
  { title: "Single keys", text: "Start, stop, update, remove and open a terminal into the agent." },
];

export const appendix = [
  { id: "a-questions", title: "Questions before an agent goes live" },
  { id: "a-role", title: "What a role defines" },
  { id: "a-egress", title: "Network access is an allowlist" },
  { id: "a-keys", title: "Credentials stay on the host" },
  { id: "a-terminals", title: "Terminals through your SSH CA" },
  { id: "a-sso", title: "Browser access through single sign-on" },
  { id: "a-updates", title: "Updates and secret rotation" },
  { id: "a-architecture", title: "Architecture" },
  { id: "a-changes", title: "What each change affects" },
  { id: "a-ui", title: "The terminal console" },
  { id: "a-cases", title: "Where teams use reef" },
  { id: "a-stack", title: "Works with what you already run" },
  { id: "a-integration", title: "Clawbits: what the integration uses" },
  { id: "a-git", title: "Clawbits: how the systems communicate" },
  { id: "a-status", title: "Status and limits" },
  { id: "a-docs", title: "Documentation" },
];
