import type { MarkdownInstance } from "astro";

export const description = "Run OpenClaw agents from one reviewed file.";

const slug = "skalenetwork/microsandbox-reef";

export const repo = `https://github.com/${slug}`;

export const latest = `https://img.shields.io/github/v/release/${slug}.json`;

export const url = (path: string) => new URL(path, import.meta.env.SITE);

export const install = `curl -fsSL ${url("/install")} | sh`;

export const bullets = [
  `A role is a [small TOML file](${repo}/blob/main/roles/hermes.toml): the image, the domains the agent may reach and the secrets it may spend. The secret values never enter the VM.`,
  "Each agent runs in its own [microsandbox](https://microsandbox.dev) microVM on your own servers and can only reach the domains its role allows.",
  "Developers create agents from the roles you approved with one command. There is no daemon or server to run.",
];

export const files = import.meta.glob(
  ["../../install.sh", "../../README.md", "../../ARCHITECTURE.md", "../../docs/*/*.md", "../../blog/*.md", "../../roles/*.toml", "../../fleet/*.toml"],
  { query: "?raw", import: "default", eager: true },
);

const DATE = /blog\/(\d{4}-\d\d-\d\d)-/;

export const route = (file: string) =>
  file.replace("../../", "").replace("install.sh", "install").replace(DATE, "blog/");

const ORDER = [
  "setup/host",
  "agents/openclaw",
  "agents/hermes",
  "enterprise/team",
  "enterprise/cloudflare-access",
  "enterprise/scopes",
  "enterprise/terminals",
];

const plain = (markdown: string) => markdown.replace(/\[([^\]]+)\]\([^)]*\)/g, "$1");

export const docs = ORDER.map((name) => {
  const [heading, summary] = files[`../../docs/${name}.md`]?.split("\n\n") ?? [];
  if (!heading?.startsWith("# ") || !summary)
    throw new Error(`docs/${name}.md: expected "# Title", a blank line, then a summary paragraph`);
  return {
    path: `/docs/${name}`,
    section: name.split("/")[0]!.replace(/^./, (c) => c.toUpperCase()),
    title: heading.slice(2),
    summary: plain(summary.trim().replace(/\n/g, " ")),
  };
});

const rendered = import.meta.glob<MarkdownInstance<Record<string, unknown>>>("../../blog/*.md", { eager: true });

export const posts = Object.entries(files)
  .filter(([file]) => file.includes("/blog/"))
  .sort(([a], [b]) => b.localeCompare(a))
  .map(([file, body]) => {
    const [heading, summary] = body.split("\n\n");
    const date = file.match(DATE)?.[1];
    if (!date || !summary || !heading.startsWith("# "))
      throw new Error(`${file}: expected blog/YYYY-MM-DD-slug.md opening with "# Title", a blank line, then a summary paragraph`);
    return {
      path: `/${route(file).replace(".md", "")}`,
      date,
      title: heading.slice(2),
      summary: summary.trim().replace(/\n/g, " "),
      md: rendered[file]!,
    };
  });
