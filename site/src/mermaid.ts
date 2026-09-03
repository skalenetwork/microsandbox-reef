import { renderMermaidSVG } from "beautiful-mermaid";
import type { MdastPluginDefinition } from "satteri";

const options = {
  bg: "var(--paper, #fff)",
  fg: "var(--text, #000)",
  accent: "var(--brand, #ca342b)",
  font: "Geist Mono",
  transparent: true,
};

const IMPORT = /^\s*@import url\([^)]*\);$/m;

export const mermaid: MdastPluginDefinition = {
  name: "mermaid",
  code(node, context) {
    if (node.lang !== "mermaid") return;
    const svg = renderMermaidSVG(node.value, options).replace(IMPORT, "");
    context.replaceNode(node, { type: "html", value: `<figure class="diagram">${svg}</figure>` });
  },
};
