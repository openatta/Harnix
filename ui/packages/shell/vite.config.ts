/**
 * One build of the whole interface (harnix-sdk.md §22.5): Harnix's shell, its first-party UI
 * packages and, for an application, its spaces' interface modules, into the directory the binary
 * embeds.
 *
 *   HARNIX_UI_FIRST_PARTY  which of Harnix's own UI packages go in (`@harnix/<name>`), comma-
 *                      separated; all of them when unset, none when empty. Each names the
 *                      backend extension it draws (`export const extension`) and registers only
 *                      while that extension is enabled, so one an application does not link costs
 *                      bytes, not behaviour.
 *   HARNIX_UI_MODULES  the application's interface entry points, comma-separated absolute paths,
 *                      in the order of its modules; each exports `register(registry, host)`.
 *                      After the first-party packages in `virtual:harnix-modules`: a later
 *                      registration wins a claim. Empty for Harnix itself.
 *   HARNIX_UI_OUT      where the build goes; `ui/dist` by default. The binary is built with
 *                      `HARNIX_UI_DIST` pointing at the same place.
 *
 * esbuild compiles the JSX; no React plugin is needed for a production build. Everything is
 * emitted as files — no inline style, no data: module — because the page is served under
 * `script-src 'self'; style-src 'self'`. React is deduplicated: an application's modules and the
 * shell must share one copy, or hooks stop working.
 *
 * The one inline script is the import map (concept §5.8, contribution API 2): an installed
 * package's precompiled module imports `react`, `react-dom`, `react/jsx-runtime` and
 * `@harnix/react` by name, and the map sends each name to an entry of this same build — a facade
 * over the very module the shell runs on, so there is still one React on the page. The server
 * allows the map by its hash (`page_csp`); nothing else inline is.
 */

import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { defineConfig, type Plugin } from "vite";

/** What the page shares with packages, by the name a package imports it by. */
const SHARED: Record<string, string> = {
  react: "shared/react",
  "react-dom": "shared/react-dom",
  "react-dom/client": "shared/react-dom-client",
  "react/jsx-runtime": "shared/react-jsx-runtime",
  "@harnix/react": "shared/harnix-react",
};

/**
 * One facade per shared name. React ships CommonJS, so its names are listed explicitly — read
 * off the module itself at build time — rather than left to `export *`, which a CommonJS module
 * cannot be relied on to answer.
 */
function sharedModules(): Plugin {
  const require = createRequire(import.meta.url);
  const prefix = "\0harnix-shared:";
  return {
    name: "harnix-shared",
    resolveId(id) {
      return id.startsWith("harnix-shared:") ? `\0${id}` : null;
    },
    load(id) {
      if (!id.startsWith(prefix)) return null;
      const name = id.slice(prefix.length);
      if (name.startsWith("@harnix/")) return `export * from ${JSON.stringify(name)};\n`;
      const names = Object.keys(require(name) as object).filter((key) => key !== "default" && /^[A-Za-z_$][\w$]*$/.test(key));
      return `import m from ${JSON.stringify(name)};\nexport default m;\nexport const { ${names.join(", ")} } = m;\n`;
    },
  };
}

/** The import map, written into the page from the files this build actually emitted. */
function importMap(): Plugin {
  return {
    name: "harnix-import-map",
    transformIndexHtml: {
      order: "post",
      handler(_html, ctx) {
        const imports: Record<string, string> = {};
        for (const chunk of Object.values(ctx.bundle ?? {})) {
          if (chunk.type !== "chunk" || !chunk.isEntry || !chunk.facadeModuleId?.startsWith("\0harnix-shared:")) continue;
          imports[chunk.facadeModuleId.slice("\0harnix-shared:".length)] = `/${chunk.fileName}`;
        }
        if (!Object.keys(imports).length) return [];
        return [{ tag: "script", attrs: { type: "importmap" }, children: JSON.stringify({ imports }), injectTo: "head-prepend" }];
      },
    },
  };
}

const VIRTUAL = "virtual:harnix-modules";

/**
 * Harnix's own UI packages, in the order they register: the settings sections in the order the
 * panel lists them, then the spaces, the system space before what fills its points.
 */
const FIRST_PARTY = ["settings", "plugins", "devices", "harnix-app", "system", "observability", "flow"];

const list = (value: string) =>
  value
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);

function harnixModules(): Plugin {
  const chosen = process.env.HARNIX_UI_FIRST_PARTY;
  const firstParty = (chosen === undefined ? FIRST_PARTY : list(chosen)).map((name) => {
    if (!FIRST_PARTY.includes(name)) throw new Error(`HARNIX_UI_FIRST_PARTY: no first-party UI package \`${name}\` (${FIRST_PARTY.join(", ")})`);
    return `@harnix/${name}`;
  });
  const entries = [...firstParty, ...list(process.env.HARNIX_UI_MODULES ?? "")];
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
  plugins: [harnixModules(), sharedModules(), importMap()],
  resolve: { dedupe: ["react", "react-dom", "@harnix/react", "@harnix/core"] },
  build: {
    rollupOptions: {
      input: {
        index: fileURLToPath(new URL("./index.html", import.meta.url)),
        ...Object.fromEntries(Object.entries(SHARED).map(([name, file]) => [file, `harnix-shared:${name}`])),
      },
      // The shared entries are imported by packages: they keep every export they declare.
      preserveEntrySignatures: "strict",
    },
    outDir: process.env.HARNIX_UI_OUT ?? fileURLToPath(new URL("../../dist", import.meta.url)),
    emptyOutDir: true,
    assetsInlineLimit: 0,
    modulePreload: { polyfill: false },
    target: "es2022",
    sourcemap: false,
  },
});
