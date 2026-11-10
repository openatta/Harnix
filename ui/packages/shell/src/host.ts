/**
 * The interface's `useHost()` host (concept §5.6), built from the app: calls limited to what this
 * page may reach, topics by name, the open session, the spaces, the composer, the right column,
 * the settings panel, the events, navigation across spaces, the shell's dialogs, the extensions
 * and hello's defaults.
 *
 * Topics go through the page's one counted table (`topics.ts`).
 */

import { parseSessionEvent, type Notifications } from "@harnix/core";
import { MAIN_INSTANCE } from "@harnix/core/state";
import { t, type HostSessionRef as SessionRef, type HostApi } from "@harnix/react";

import { disabledOf, later, orderTabs, safeTabs, type App } from "./app.ts";
import { sessionMenu } from "./dialogs.tsx";
import { hostDialogs } from "./host-dialogs.tsx";
import { menuBelow } from "./Menu.tsx";
import { revealEntry } from "./reveal.ts";
import { openSettings, settingsSections } from "./settings.tsx";
import { topicsOf } from "./topics.ts";

export function hostFor(app: App, owner = "page"): HostApi {
  const listed = (method: string) => {
    const reachable = app.snapshot.reachable;
    return !reachable.length || reachable.includes(method) || method.startsWith("harnix.topics.subscribe") || method.startsWith("harnix.topics.unsubscribe");
  };
  const call = async <T,>(method: string, params?: unknown): Promise<T> => {
    // A runtime application's methods come and go while the page is open (installed, enabled):
    // one not listed yet is asked about once more before it is refused here.
    if (!listed(method) && method.startsWith("app.")) await app.refreshReachable();
    if (!listed(method)) throw new Error(t("contrib.methodNotDeclared", { owner, method }));
    return app.call<T>(method, params);
  };
  const spaceIds = () => app.registry.at("space").map((c) => c.id);
  const switchTo = async (id: string): Promise<void> => {
    if (!spaceIds().includes(id)) throw new Error(t("host.noSpace", { space: id }));
    // A space that cannot be entered for now (`disabled()`, ipdrun-48): no switch, no error — the
    // caller reads `spaces.current()` for where it is.
    if (disabledOf(app.registry, app.registry.at("space").find((c) => c.id === id)) !== null) return;
    await app.switchSpace(id);
    await app.afterDraw();
  };
  const frontTabs = () => orderTabs(safeTabs(app.registry, app.registry.at("space").find((c) => c.id === app.snapshot.space)));
  return {
    call,
    subscribe(topic, params, fn) {
      return topicsOf(app.conn).subscribe(topic, params ?? {}, fn);
    },
    session: {
      current: () => {
        const current = app.snapshot.current;
        return current ? { instance: current.instance, sessionId: current.sessionId } : null;
      },
      context: () => app.controller?.getSnapshot() ?? null,
      list: () => app.snapshot.sessions,
      open: ((ref: SessionRef, options?: { switchSpace?: boolean; reveal?: string }) => {
        const opened = !options?.switchSpace
          ? app.open(ref)
          : (async () => {
              const space = await spaceOfSession(app, ref);
              await switchTo(space);
              app.open({ instance: ref.instance, sessionId: ref.sessionId }, space);
              await app.afterDraw();
            })();
        if (options?.reveal === undefined) return opened;
        const key = options.reveal;
        return (async () => {
          await opened;
          return revealEntry(app, { instance: ref.instance, sessionId: ref.sessionId }, key);
        })();
      }) as HostApi["session"]["open"],
      reveal: async (key) => {
        const current = app.snapshot.current;
        return current ? revealEntry(app, current, key) : false;
      },
      async create(spec = {}) {
        await app.create(spec);
        const current = app.snapshot.current!;
        return { instance: current.instance ?? MAIN_INSTANCE, sessionId: current.sessionId };
      },
      onChange: (fn) => app.onCurrent(fn),
      isOpen: (ref) => Object.values(app.snapshot.currentBySpace).some((k) => k != null && k.instance === ref.instance && k.sessionId === ref.sessionId),
      menu(ref, anchor) {
        const origin = anchor instanceof HTMLElement ? anchor : null;
        const at = anchor instanceof Element ? menuBelow(anchor) : anchor;
        sessionMenu(app, { instance: ref.instance, session_id: ref.sessionId, name: ref.name ?? null }, { origin, at });
      },
    },
    spaces: {
      switch: switchTo,
      current: () => app.snapshot.space,
    },
    instances: {
      list: () => call("harnix.instances.list", {}),
      sessions: (instance) => call("harnix.sessions.list", { instance }),
      tasks: (instance, sessionId) => call("harnix.session.tasks", { instance, session_id: sessionId }),
      create: (template, name, params) => call("harnix.instances.create", { template, name, params }),
      stop: (instance) => call("harnix.instances.stop", { instance }),
    },
    composer: {
      insert: (text, options) => app.insert(text, options?.session),
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
        const ids = settingsSections(app);
        openSettings(app, section !== undefined && ids.includes(section) ? section : ids[0]);
      },
    },
    shell: {
      badge: (n) => app.setTitleBadge(n),
      context: (c) => app.setShellContext(c),
      notice: (text, tone) => app.notice(text, tone === "error" ? "error" : "info"),
    },
    modal: {
      isOpen: () => app.snapshot.modal != null,
      onChange: (fn) => app.onModal(fn),
    },
    redraw: () => app.redraw(),
    ext: (id) => ({
      fetch: (path, init) => app.conn.appFetch(id, path, init),
      socket: (path) => app.conn.appSocket(id, path) as unknown as WebSocket,
    }),
    reveal: (item) => app.showDetail(item),
    t,
    events: {
      onTurnSettled: (fn) =>
        app.conn.onNotification((method, params) => {
          if (method === "harnix.turn_settled") fn(params as unknown as Notifications["harnix.turn_settled"]);
        }),
      onSessionEvent: (ref, fn) =>
        app.conn.onNotification((method, params) => {
          if (method !== "harnix.event") return;
          const frame = params as unknown as Notifications["harnix.event"];
          if (frame.instance !== ref.instance || frame.session_id !== ref.sessionId) return;
          fn({ raw: frame.event, parsed: parseSessionEvent(frame.event), trust: frame.trust === "extension" ? "extension" : "product" });
        }),
      onHostEvent: (fn) =>
        app.conn.onNotification((method, params) => {
          if (method === "harnix.host_event") fn(params as unknown as Notifications["harnix.host_event"]);
        }),
    },
    navigate: {
      async open(target) {
        await switchTo(target.space);
        if (app.snapshot.space !== target.space) return;
        if (target.session) {
          app.open({ instance: target.session.instance, sessionId: target.session.sessionId }, target.space);
          await app.afterDraw();
        }
        if (target.selection === undefined) return;
        const space = app.registry.at("space").find((c) => c.id === target.space);
        if (!space?.accept) return;
        try {
          space.accept(target.selection);
        } catch (e) {
          later(() => app.registry.fail("space", { id: space.id, owner: space.owner }, e));
        }
      },
    },
    dialogs: hostDialogs(app),
    extensions: {
      list: () => app.snapshot.extensions.map((e) => ({ ...e })),
      onChange: (fn) => app.onExtensions(fn),
    },
    get defaults() {
      const given = app.snapshot.hello?.defaults;
      return { instance: given?.instance ?? MAIN_INSTANCE, space: given?.space ?? null };
    },
  };
}

/**
 * The interface space a session opens in: the front one if it lists the backend space the hub has
 * the session in, else the first that does. The shell's own list cannot say — it lists the session
 * space alone — so the hub is asked.
 */
async function spaceOfSession(app: App, ref: { instance: string; sessionId: string }): Promise<string> {
  const { sessions } = await app.call<{ sessions: { session_id: string; space?: string }[] }>("harnix.sessions.list", { instance: ref.instance });
  const row = sessions.find((r) => r.session_id === ref.sessionId);
  if (!row) throw new Error(t("host.noSession", { session: `${ref.instance}/${ref.sessionId}` }));
  const backend = row.space ?? app.snapshot.hello?.defaults?.space ?? app.claim();
  if (app.claim() === backend) return app.snapshot.space;
  const spaces = [...app.registry.at("space")].sort((a, b) => (a.order ?? 100) - (b.order ?? 100));
  const space = spaces.find((c) => c.id === backend) ?? spaces.find((c) => app.claim(c.id) === backend);
  if (!space) throw new Error(t("host.noSpace", { space: backend }));
  return space.id;
}
