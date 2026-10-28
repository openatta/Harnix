/**
 * The interface's `useNest()` host (design §10.4), built from the app: calls limited to what this
 * page may reach, topics by name, the open session, the spaces, the composer, the right column,
 * the settings panel.
 *
 * Topics go through the page's one counted table (`topics.ts`).
 */

import { SYSTEM_INSTANCE } from "@harnix/core/state";
import { t, type NestHost } from "@harnix/react";

import { later, orderTabs, safeTabs, type App } from "./app.ts";
import { openSettings } from "./settings.tsx";
import { topicsOf } from "./topics.ts";

export function hostFor(app: App, owner = "page"): NestHost {
  const call = <T,>(method: string, params?: unknown): Promise<T> => {
    const reachable = app.snapshot.reachable;
    if (reachable.length && !reachable.includes(method) && !method.startsWith("harnix.subscribe") && !method.startsWith("harnix.unsubscribe")) {
      return Promise.reject(new Error(t("contrib.methodNotDeclared", { owner, method })));
    }
    return app.call<T>(method, params);
  };
  const spaceIds = () => app.registry.at("space").map((c) => c.id);
  const switchTo = async (id: string): Promise<void> => {
    if (!spaceIds().includes(id)) throw new Error(t("host.noSpace", { space: id }));
    await app.switchSpace(id);
    await app.afterDraw();
  };
  const frontTabs = () => orderTabs(safeTabs(app.registry, app.registry.at("space").find((c) => c.id === app.snapshot.space)));
  return {
    call,
    subscribe(topic, params, fn) {
      return topicsOf(app.client).subscribe(topic, params ?? {}, fn);
    },
    session: {
      current: () => {
        const current = app.snapshot.current;
        return current ? { instance: current.instance, sessionId: current.sessionId } : null;
      },
      context: () => app.controller?.getSnapshot() ?? null,
      list: () => app.snapshot.sessions,
      open: (ref, options) => {
        if (!options?.switchSpace) return app.open(ref);
        return (async () => {
          const space = await spaceOfSession(app, ref);
          await switchTo(space);
          app.open({ instance: ref.instance, sessionId: ref.sessionId }, space);
          await app.afterDraw();
        })();
      },
      async create(spec = {}) {
        await app.create(spec);
        const current = app.snapshot.current!;
        return { instance: current.instance ?? SYSTEM_INSTANCE, sessionId: current.sessionId };
      },
      onChange: (fn) => app.onCurrent(fn),
    },
    spaces: {
      switch: switchTo,
      current: () => app.snapshot.space,
    },
    instances: {
      list: () => call("harnix.instances.list", {}),
      sessions: (instance) => call("harnix.instances.sessions", { instance }),
      tasks: (instance, sessionId) => call("harnix.sessions.tasks", { instance, session_id: sessionId }),
      create: (template, name, params) => call("harnix.instances.create", { template, name, params }),
      stop: (instance) => call("harnix.instances.stop", { instance }),
    },
    composer: {
      insert: (text) => app.composer?.insert(text),
    },
    details: {
      open(tab) {
        app.toggleDetails(true);
        if (tab === undefined) return;
        // A tab the space's `tabs()` gives only now: read them again before giving up on it.
        let known = tab === "details" || frontTabs().some((x) => x.id === tab);
        if (!known) {
          app.redraw();
          known = frontTabs().some((x) => x.id === tab);
        }
        if (known) app.selectTab(tab, true);
        else later(() => app.registry.note({ point: "details", owner, id: tab, reason: t("host.noTab", { tab, space: app.snapshot.space }) }));
      },
      close: () => app.toggleDetails(false),
      isOpen: () => app.detailsOpen(),
      tab: () => app.shownTab(),
      onTab: (fn) => app.onTab(fn),
    },
    settings: {
      open(section) {
        const ids = app.registry.at("settings.section").map((s) => s.id);
        openSettings(app, section !== undefined && ids.includes(section) ? section : ids[0]);
      },
    },
    redraw: () => app.redraw(),
    space: (module) => ({
      fetch: (path, init) => app.client.appFetch(module, path, init),
      socket: (path) => app.client.appSocket(module, path) as unknown as WebSocket,
    }),
    reveal: (item) => app.showDetail(item),
    t,
  };
}

/**
 * The interface space a session opens in: the front one if it lists the backend space the hub has
 * the session in, else the first that does. The shell's own list cannot say — it lists the session
 * space alone — so the hub is asked.
 */
async function spaceOfSession(app: App, ref: { instance: string; sessionId: string }): Promise<string> {
  const { sessions } = await app.call<{ sessions: { session_id: string; space?: string }[] }>("harnix.sessions", { instance: ref.instance });
  const row = sessions.find((r) => r.session_id === ref.sessionId);
  if (!row) throw new Error(t("host.noSession", { session: `${ref.instance}/${ref.sessionId}` }));
  const backend = row.space ?? "sessions";
  if (app.claim() === backend) return app.snapshot.space;
  const spaces = [...app.registry.at("space")].sort((a, b) => (a.order ?? 100) - (b.order ?? 100));
  const space = spaces.find((c) => c.id === backend) ?? spaces.find((c) => app.claim(c.id) === backend);
  if (!space) throw new Error(t("host.noSpace", { space: backend }));
  return space.id;
}
