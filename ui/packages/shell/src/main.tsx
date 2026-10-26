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
import { Shell } from "./Shell.tsx";

const client = new HarnixClient({ token: tokenFromDocument() });
const app = new App(client, new Registry());
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
// The app and its client, for the console and for tests that drive the page from inside it. Nothing
// a same-origin script could not already reach: the token is in the page.
(window as unknown as { harnix: unknown }).harnix = { app, client, host };

createRoot(document.getElementById("root")!).render(<Root />);
// The built-ins first, then the application's modules in its order: a later registration wins a
// claim, so an application can take over what Harnix draws.
app.start((a) => {
  registerBuiltins(a);
  for (const module of modules) {
    try {
      module.register?.(a.registry, host);
    } catch (e) {
      a.registry.note({ point: "module", owner: "application", reason: e instanceof Error ? e.message : String(e) });
    }
  }
});
window.addEventListener("beforeunload", () => {
  const current = app.snapshot.current;
  if (current) client.notify("harnix.detach", { instance: current.instance, session_id: current.sessionId });
});
