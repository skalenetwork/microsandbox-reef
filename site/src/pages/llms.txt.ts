import type { APIRoute } from "astro";
import { description, docs, files, install, intro, posts, quickstart, route, url } from "../content";

export const GET: APIRoute = () => {
  const examples = Object.keys(files)
    .map(route)
    .filter((path) => path.endsWith(".toml"))
    .map((path) => `- [${path}](${url(path)})`);
  const guides = docs.map((doc) => `- [${doc.title}](${url(`${doc.path}.md`)}): ${doc.summary}`);
  const articles = posts.map((post) => `- [${post.title}](${url(`${post.path}.md`)}): ${post.date}. ${post.summary}`);
  const optional = posts.length ? `\n## Optional\n\n${articles.join("\n")}\n` : "";
  return new Response(`# reef

> ${description}

${intro}

reef runs on Linux x86_64/aarch64 with KVM and glibc 2.39 or newer, and on Apple Silicon macOS. Every agent is a microsandbox microVM, so the host also needs \`msb\`, the microsandbox runtime, at the release this reef pins.

## Start

\`\`\`sh
curl -fsSL https://install.microsandbox.dev | sh
${install}
reef doctor
\`\`\`

The first agent is an OpenClaw gateway that picks its model provider in the browser:

\`\`\`sh
${quickstart}
\`\`\`

## Docs

- [README](${url("/README.md")}): every command, role and fleet file format, secrets.toml, state, known limits
- [Architecture](${url("/ARCHITECTURE.md")}): goal, model, invariants, what is deliberately absent
${guides.join("\n")}
- [Source](https://github.com/skalenetwork/microsandbox-reef): MIT, SKALE Labs

## Examples

- [roles/README.md](${url("/roles/README.md")}): what each example role is for
${examples.join("\n")}
${optional}`);
};
