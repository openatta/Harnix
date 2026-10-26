/**
 * One build of the whole interface (design §11.4): Harnix's shell and, for an application, its
 * spaces' interface modules, into the directory the binary embeds.
 *
 *   HARNIX_UI_MODULES  the application's interface entry points, comma-separated absolute paths,
 *                      in the order of its modules; each exports `register(registry, host)`.
 *                      Generated into `virtual:harnix-modules`. Empty for Harnix itself.
 *   HARNIX_UI_OUT      where the build goes; `ui/dist` by default. The binary is built with
 *                      `HARNIX_UI_DIST` pointing at the same place.
 *
 * esbuild compiles the JSX; no React plugin is needed for a production build. Everything is
 * emitted as files — no inline script, no inline style, no data: module — because the page is
 * served under `script-src 'self'; style-src 'self'`. React is deduplicated: an application's
 * modules and the shell must share one copy, or hooks stop working.
 */

import { fileURLToPath } from "node:url";
import { defineConfig, type Plugin } from "vite";

const VIRTUAL = "virtual:harnix-modules";

function harnixModules(): Plugin {
  const entries = (process.env.HARNIX_UI_MODULES ?? "")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
  return {
    name: "harnix-modules",
    resolveId(id) {
      return id === VIRTUAL ? `\0${VIRTUAL}` : null;
    },
    load(id) {
      if (id !== `\0${VIRTUAL}`) return null;
      const imports = entries.map((path, i) => `import * as m${i} from ${JSON.stringify(path)};`);
      return `${imports.join("\n")}\nexport default [${entries.map((_, i) => `m${i}`).join(", ")}];\n`;
    },
  };
}

export default defineConfig({
  root: fileURLToPath(new URL(".", import.meta.url)),
  base: "/",
  esbuild: { jsx: "automatic" },
  plugins: [harnixModules()],
  resolve: { dedupe: ["react", "react-dom", "@harnix/react", "@harnix/core"] },
  build: {
    outDir: process.env.HARNIX_UI_OUT ?? fileURLToPath(new URL("../../dist", import.meta.url)),
    emptyOutDir: true,
    assetsInlineLimit: 0,
    modulePreload: { polyfill: false },
    target: "es2022",
    sourcemap: false,
  },
});
