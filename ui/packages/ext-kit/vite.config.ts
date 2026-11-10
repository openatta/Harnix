/**
 * The kit as one classic script and one stylesheet (`dist/harnix-ext-kit.js`, global `HarnixKit`;
 * `dist/harnix-ext-kit.css`: the shell's tokens and the components), for an application that ships
 * its page unbundled. A bundling application imports `@harnix/ext-kit` and `@harnix/ext-kit/kit.css`.
 */

import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

export default defineConfig({
  build: {
    lib: {
      entry: fileURLToPath(new URL("./src/global.ts", import.meta.url)),
      name: "HarnixKit",
      formats: ["iife"],
      fileName: () => "harnix-ext-kit.js",
      cssFileName: "harnix-ext-kit",
    },
    outDir: fileURLToPath(new URL("./dist", import.meta.url)),
    emptyOutDir: true,
    target: "es2022",
    minify: false,
    cssMinify: false,
    sourcemap: false,
  },
});
