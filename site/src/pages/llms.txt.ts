import type { APIRoute } from "astro";
import { bullets, description, docs, files, install, route, url } from "../content";

export const GET: APIRoute = () => {
  const examples = Object.keys(files)
    .map(route)
    .filter((path) => path.endsWith(".toml"))
    .map((path) => `- [${path}](${url(path)})`);
  const guides = docs.map((doc) => `- [${doc.title}](${url(`${doc.path}.md`)})`);
  return new Response(`# reef

> ${description}

${bullets.map((bullet) => `- ${bullet}`).join("\n")}

Install: \`${install}\`, then \`reef doctor\`. Linux x86_64/aarch64 and Apple Silicon macOS.

## Docs

- [README](${url("/README.md")}): every command, role and fleet file format, secrets.toml, state, known limits
- [Architecture](${url("/ARCHITECTURE.md")}): goal, model, invariants, what is deliberately absent
${guides.join("\n")}
- [Source](https://github.com/skalenetwork/reef): MIT, SKALE Labs

## Examples

${examples.join("\n")}
`);
};
