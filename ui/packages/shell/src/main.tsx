/**
 * The interface's entry: one client, one registry, one app; built-ins registered on first connect.
 *
 * The token is in `<meta name="harnix-token">`, put there by the server when it serves this page
 * (empty when the browser was admitted by a launch link and holds a cookie instead).
 */

import { createRoot } from "react-dom/client";
import { HarnixClient, tokenFromDocument } from "@harnix/core";
import { HarnixUiProvider, NestProvider, Registry, currentLocale, en, useLocale, zhCN } from "@harnix/react";

import "@harnix/react/tokens.css";
import "@harnix/react/styles.css";
import "./styles/index.css";

import modules from "virtual:harnix-modules";

import { App } from "./app.ts";
import { registerBuiltins } from "./builtins.tsx";
import { hostFor } from "./host.ts";
import { iconsFrom } from "./icons.ts";
import { loadPackages } from "./packages.ts";
import { Shell } from "./Shell.tsx";

/**
 * "overlay" in the desktop shell on macOS (templates/desktop): the window has no title bar, its
 * traffic lights sit over the page's top-left, and the top bars are where it is dragged. The shell
 * is known by the internals Tauri puts in every page it opens; nothing is asked of it.
 */
function chrome(): string {
  const tauri = "__TAURI_INTERNALS__" in window;
  return tauri && /Mac OS X|Macintosh/.test(navigator.userAgent) ? "overlay" : "";
}

const client = new HarnixClient({ token: tokenFromDocument() });
const app = new App(client, new Registry());
iconsFrom((name) => app.registry.icon(name));
const host = hostFor(app);

function Root() {
  const locale = useLocale();
  const hello = app.snapshot.hello;
  return (
    <HarnixUiProvider client={client} strings={locale === "en" ? en : zhCN} promptTimeoutSecs={hello?.limits?.prompt_timeout_secs}>
      <NestProvider host={host}>
        <Shell app={app} />
      </NestProvider>
    </HarnixUiProvider>
  );
}

document.documentElement.lang = currentLocale();
document.documentElement.dataset.chrome = chrome();
// The app and its client, for the console and for tests that drive the page from inside it. Nothing
// a same-origin script could not already reach: the token is in the page.
(window as unknown as { harnix: unknown }).harnix = { app, client, host };

createRoot(document.getElementById("root")!).render(<Root />);
// The built-ins first, then the application's modules in its order, then installed packages' v2
// modules: a later registration wins a claim, so an application can take over what Harnix draws.
app.start((a) => {
  registerBuiltins(a);
  for (const module of modules) {
    try {
      module.register?.(a.registry, host);
    } catch (e) {
      a.registry.note({ point: "module", owner: "application", reason: e instanceof Error ? e.message : String(e) });
    }
  }
}, loadPackages);
window.addEventListener("beforeunload", () => {
  const current = app.snapshot.current;
  if (current) client.notify("harnix.detach", { instance: current.instance, session_id: current.sessionId });
});
