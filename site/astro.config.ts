import { defineConfig, fontProviders } from "astro/config";

export default defineConfig({
  site: "https://reef.clawbits.ai",
  build: { format: "file" },
  devToolbar: { enabled: false },
  markdown: { shikiConfig: { theme: "css-variables" } },
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
