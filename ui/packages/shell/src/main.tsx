/**
 * The interface's entry: one client, one registry, one app; on first connect the built-ins and
 * the UI packages hello says are enabled, before the first draw; then whatever `harnix.ui` says.
 *
 * The token is in `<meta name="harnix-token">`, put there by the server when it serves this page
 * (empty when the browser was admitted by a launch link and holds a cookie instead).
 */

import { createRoot } from "react-dom/client";
import { HarnixClient, tokenFromDocument, tokenFromPage } from "@harnix/core";
import { HarnixUiProvider, HostProvider, Registry, RegistryProvider, currentLocale, en, useLocale, zhCN } from "@harnix/react";

import "@harnix/react/tokens.css";
import "@harnix/react/styles.css";
import "./styles/index.css";

import modules from "virtual:harnix-modules";

import { activate } from "./activation.ts";
import { App } from "./app.ts";
import { registerBuiltins } from "./builtins.tsx";
import { hostFor } from "./host.ts";
import { iconsFrom } from "./icons.ts";
import { followModules } from "./packages.ts";
import { Shell } from "./Shell.tsx";
import { followSurfaces } from "./surfaces/register.tsx";

/**
 * "overlay" in the desktop shell on macOS (templates/desktop): the window has no title bar, its
 * traffic lights sit over the page's top-left, and the top bars are where it is dragged. The shell
 * is known by the internals Tauri puts in every page it opens; nothing is asked of it.
 */
function chrome(): string {
  const tauri = "__TAURI_INTERNALS__" in window;
  return tauri && /Mac OS X|Macintosh/.test(navigator.userAgent) ? "overlay" : "";
}

// After a restart the backend has a new token, and the old one is refused: the page asks itself for
// the current one again rather than staying disconnected until someone reloads it.
const client = new HarnixClient({ token: tokenFromDocument(), refreshToken: tokenFromPage() });
const app = new App(client, new Registry());
iconsFrom((name) => app.registry.icon(name));
const host = hostFor(app);

function Root() {
  const locale = useLocale();
  const hello = app.snapshot.hello;
  return (
    <HarnixUiProvider client={client} strings={locale === "en" ? en : zhCN} promptTimeoutSecs={hello?.limits?.prompt_timeout_secs}>
      <HostProvider host={host}>
        <RegistryProvider registry={app.registry}>
          <Shell app={app} />
        </RegistryProvider>
      </HostProvider>
    </HarnixUiProvider>
  );
}

document.documentElement.lang = currentLocale();
document.documentElement.dataset.chrome = chrome();
// The app and its client, for the console and for tests that drive the page from inside it. Nothing
// a same-origin script could not already reach: the token is in the page.
(window as unknown as { harnix: unknown }).harnix = { app, client, host };

createRoot(document.getElementById("root")!).render(<Root />);
// The built-ins first, then the compiled-in UI packages in the application's order — those for an
// extension only while it is enabled — then installed packages' modules and frames: a later
// registration wins a claim, so an application can take over what Harnix draws.
app.start(
  async (a) => {
    await registerBuiltins(a);
    activate(a, host, modules, a.snapshot.extensions);
  },
  async (a, view) => {
    activate(a, host, modules, view.extensions);
    await followModules(a, view);
    await followSurfaces(a, view);
    // What this round registered into a point nobody defined goes now (`orElse`, or the diagnostics).
    a.registry.settle();
  },
);
window.addEventListener("beforeunload", () => {
  const current = app.snapshot.current;
  if (current) client.notify("harnix.session.detach", { instance: current.instance, session_id: current.sessionId });
});
