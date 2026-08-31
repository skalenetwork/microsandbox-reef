import { defineConfig, fontProviders } from "astro/config";
import { satteri } from "@astrojs/markdown-satteri";
import { mermaid } from "./src/mermaid";

export default defineConfig({
  site: "https://reef.clawbits.ai",
  build: { format: "file" },
  devToolbar: { enabled: false },
  markdown: {
    shikiConfig: { theme: "css-variables" },
    processor: satteri({ mdastPlugins: [mermaid] }),
  },
  fonts: [
    {
      provider: fontProviders.fontsource(),
      name: "Geist Mono",
      cssVariable: "--font",
      weights: ["400 500"],
      styles: ["normal"],
      fallbacks: ["monospace"],
    },
  ],
});
