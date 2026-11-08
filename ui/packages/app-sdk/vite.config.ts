/**
 * The SDK as a classic script (`dist/harnix-app-sdk.js`, global `HarnixApp`), for an application
 * that ships its page unbundled — the showcase does (`examples/apps/showcase/ui/frame/vendor/`).
 * A bundling application imports `@harnix/app-sdk` instead.
 */

import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

export default defineConfig({
  build: {
    lib: {
      entry: fileURLToPath(new URL("./src/global.ts", import.meta.url)),
      name: "HarnixApp",
      formats: ["iife"],
      fileName: () => "harnix-app-sdk.js",
    },
    outDir: fileURLToPath(new URL("./dist", import.meta.url)),
    emptyOutDir: true,
    target: "es2022",
    minify: false,
    sourcemap: false,
  },
});
