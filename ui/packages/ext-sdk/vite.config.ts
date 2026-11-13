/**
 * The SDK as a classic script (`dist/harnix-ext-sdk.js`, global `HarnixExt`), for an application
 * that ships its page unbundled — the showcase does (`examples/packages/showcase/ui/frame/vendor/`).
 * A bundling application imports `@harnix/ext-sdk` instead.
 */

import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

export default defineConfig({
  build: {
    lib: {
      entry: fileURLToPath(new URL("./src/global.ts", import.meta.url)),
      name: "HarnixExt",
      formats: ["iife"],
      fileName: () => "harnix-ext-sdk.js",
    },
    outDir: fileURLToPath(new URL("./dist", import.meta.url)),
    emptyOutDir: true,
    target: "es2022",
    minify: false,
    sourcemap: false,
  },
});
