export const description = "Isolated computers for your org's agents.";

export const url = (path: string) => new URL(path, import.meta.env.SITE);

export const install = `curl -fsSL ${url("/install")} | sh`;

export const bullets = [
  "Each agent runs in its own microVM on your own servers and can only reach the domains its role allows.",
  "A role is a [small TOML file](https://github.com/skalenetwork/reef/blob/main/roles/hermes.toml) with the image, allowed domains and secrets. The secret values never enter the VM.",
  "Developers create agents from the roles you approved with one command. There is no daemon or server to run.",
];

export const files = import.meta.glob(
  ["../../install.sh", "../../README.md", "../../ARCHITECTURE.md", "../../docs/*/*.md", "../../roles/*.toml", "../../fleet/*.toml"],
  { query: "?raw", import: "default", eager: true },
);

export const route = (file: string) => file.replace("../../", "").replace("install.sh", "install");

export const docs = Object.entries(files)
  .filter(([file]) => file.includes("/docs/"))
  .sort(([a], [b]) => a.localeCompare(b))
  .map(([file, body]) => ({
    path: `/${route(file).replace(".md", "")}`,
    section: route(file).split("/")[1].replace(/^./, (c) => c.toUpperCase()),
    title: body.split("\n")[0].slice(2),
  }));
