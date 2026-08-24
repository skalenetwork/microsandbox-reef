import { defineConfig, fontProviders } from "astro/config";

export default defineConfig({
  site: "https://reef.clawbits.ai",
  devToolbar: { enabled: false },
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
