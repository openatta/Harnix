/**
 * The shell's end of one frame's bridge (`harnix-ui-bridge@1`, `@harnix/app-sdk/protocol`).
 *
 * The frame is handed a `MessageChannel` port once, when its page first loads, and that port is
 * the only thing the shell listens to for it: no window message is ever read. A second load is
 * the page going away (it navigated, or was replaced): the frame has failed, and is emptied at
 * once (`about:blank`) rather than left showing — and running — whatever it went to, and the
 * failure is on the Apps tab's record. Where it may go at all is the page's own `frame-src 'self'`:
 * this origin, so the address it went to carries nothing to anyone else. So has one that does
 * not say hello in time (`HELLO_MS`), and one that stops answering the heartbeat (`PING_MS`,
 * `MISSES`) — which only a browser that runs sandboxed frames apart from the page can notice: where
 * the frame shares the page's thread, a page that hangs hangs the shell with it.
 *
 * Everything the frame asks is done by the shell, and only what its application's manifest lets
 * it ask for: methods and topics of its own (`app.<id>.*`) and in its `[interface] hub` — an
 * allowlist (`FRAME_HUB`), each name narrowed to the application's own spaces and their sessions
 * before the call goes out as the page —, sessions of its own space, items of its own kinds, its
 * own modals and settings sections. A refusal is
 * answered as one, recorded for the Apps tab (`store.ts`) and noted in the registry, which
 * Settings → Diagnostics lists.
 */

import { errorText } from "@harnix/core";
import { SYSTEM_INSTANCE } from "@harnix/core/state";
import { t } from "@harnix/react";
import {
  FRAME_HUB,
  PROTOCOL,
  admits,
  inDialog,
  shellKey,
  type Connect,
  type DetailItem,
  type Environment,
  type ErrorCode,
  type FrameRequests,
  type HubScope,
  type KeyPress,
  type Message,
  type SessionRef,
  type ShellEvents,
  type Welcome,
} from "@harnix/app-sdk/protocol";

import type { App } from "../app.ts";
import { openSettings } from "../settings.tsx";
import { topicsOf } from "../topics.ts";
import { ask } from "./confirm.tsx";
import { onShellEnvironment, shellEnvironment } from "./env.ts";
import { autoHeight, titleOf, type AppInterface, type SurfaceDecl } from "./model.ts";
import { surfaces } from "./store.ts";

/** How long a page has to say hello after it has loaded. */
export const HELLO_MS = 15_000;
/** The heartbeat, and how many unanswered beats are a page that stopped answering. */
export const PING_MS = 1_000;
export const MISSES = 5;

const MAX_TEXT = 10_000;

/** What the shell does for a frame that is not the bridge's own business: the frame's view. */
export interface FrameHost {
  app: App;
  iface: AppInterface;
  decl: SurfaceDecl;
  /** The interface space the application's sessions are drawn in (its first, if it has one). */
  space: string | null;
  params: unknown;
  ready(): void;
  failed(reason: string): void;
  resize(height: number): void;
  close(result: unknown): void;
  /** One of the application's modals; resolves with what it closed with. */
  openModal(id: string, params: unknown): Promise<unknown>;
  /** The application's space's second panel, given these params (null closes it). */
  secondary(open: { params: unknown; title?: string } | null): void;
  /** A badge on the application's space entry. */
  badge(value: string | number | null): void;
}

/** A request the application may not make: answered `refused`, and recorded. */
class Refused extends Error {
  constructor(
    readonly what: string,
    message: string,
  ) {
    super(message);
  }
}

class Invalid extends Error {}

class Unknown extends Error {}

export class FrameBridge {
  private port: MessagePort | null = null;
  private loads = 0;
  private helloTimer: ReturnType<typeof setTimeout> | null = null;
  private beat: ReturnType<typeof setInterval> | null = null;
  private waitingPong = false;
  private misses = 0;
  private nextRequest = 1;
  private readonly subs = new Map<number, () => void>();
  private nextSub = 1;
  private readonly offs: (() => void)[] = [];
  private local: { visible: boolean; modal: boolean } = { visible: true, modal: false };
  private lastEnv = "";
  private lastSession = "";
  private disposed = false;

  constructor(
    private readonly iframe: HTMLIFrameElement,
    private readonly host: FrameHost,
  ) {
    iframe.addEventListener("load", this.loaded);
  }

  /** The frame's own part of the environment: whether it can be seen, whether a dialog is over it. */
  setLocal(local: { visible: boolean; modal: boolean }): void {
    this.local = local;
    this.pushEnv();
  }

  setParams(params: unknown): void {
    this.host.params = params;
    this.send({ kind: "event", name: "params", data: { params } satisfies ShellEvents["params"] });
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.iframe.removeEventListener("load", this.loaded);
    if (this.helloTimer) clearTimeout(this.helloTimer);
    if (this.beat) clearInterval(this.beat);
    for (const off of this.subs.values()) off();
    this.subs.clear();
    for (const off of this.offs.splice(0)) off();
    this.port?.close();
    this.port = null;
  }

  private fail(reason: string): void {
    if (this.disposed) return;
    this.dispose();
    this.host.failed(reason);
  }

  private readonly loaded = (): void => {
    this.loads += 1;
    if (this.loads > 1) {
      // Not the application's page any more: whatever it navigated to is not left running here.
      this.iframe.removeEventListener("load", this.loaded);
      this.iframe.src = "about:blank";
      this.fail(t("surface.failed.navigated"));
      return;
    }
    const channel = new MessageChannel();
    this.port = channel.port1;
    this.port.onmessage = (event) => this.receive(event.data as Message);
    // To this frame's window alone. Its origin is opaque, so there is no origin to name: what makes
    // it this frame's is that this is the frame's first load, which nothing else could have caused.
    this.iframe.contentWindow?.postMessage({ protocol: PROTOCOL, type: "connect" } satisfies Connect, "*", [channel.port2]);
    this.helloTimer = setTimeout(() => this.fail(t("surface.failed.noHello")), HELLO_MS);
  };

  private send(message: Message, transfer: Transferable[] = []): void {
    if (!this.disposed) this.port?.postMessage(message, transfer);
  }

  private receive(message: Message | null): void {
    if (!message || typeof message !== "object" || this.disposed) return;
    if (message.kind === "request") void this.answer(message.id, message.method, message.params);
    else if (message.kind === "response") {
      // The heartbeat is the only thing the shell asks.
      this.waitingPong = false;
      this.misses = 0;
    } else if (message.kind === "event") this.onEvent(message.name, message.data);
  }

  private async answer(id: number, method: string, params: unknown): Promise<void> {
    try {
      const result = await this.handle(method, (params ?? {}) as Record<string, unknown>);
      this.send({ kind: "response", id, result: result ?? null });
    } catch (e) {
      let code: ErrorCode = "failed";
      if (e instanceof Refused) {
        code = "refused";
        surfaces.refuse(this.host.iface.id, e.what, e.message);
        this.host.app.registry.note({ point: "bridge", owner: this.host.iface.id, id: e.what, reason: e.message });
      } else if (e instanceof Invalid) code = "invalid";
      else if (e instanceof Unknown) code = "unknown";
      this.send({ kind: "response", id, error: { code, message: errorText(e) } });
    }
  }

  private refuse(name: string, why: string): never {
    throw new Refused(name, t("surface.refused", { app: this.host.iface.id, name, why }));
  }

  private get spaceSessions(): string | null {
    const { decl, iface } = this.host;
    return decl.sessions ?? iface.spaces[0] ?? null;
  }

  private current(): SessionRef | null {
    const space = this.host.space;
    const key = space ? this.host.app.snapshot.currentBySpace[space] : null;
    return key ? { instance: key.instance, sessionId: key.sessionId } : null;
  }

  /**
   * A call through a name of `[interface] hub`, narrowed by its scope: about the sessions of one of
   * the application's own spaces (`space` filled in when it is left out), or about one session in
   * them. The page's connection could reach any session; a frame is given its own.
   */
  private async narrow(name: string, scope: HubScope | undefined, raw: unknown): Promise<unknown> {
    if (!scope || scope === "open") return raw;
    const params: Record<string, unknown> = raw && typeof raw === "object" && !Array.isArray(raw) ? { ...(raw as Record<string, unknown>) } : {};
    const own = this.host.iface.spaces;
    if (!own.length) this.refuse(name, t("surface.why.noSpace"));
    if (scope === "space") {
      const space = params.space ?? this.spaceSessions;
      if (typeof space !== "string" || !own.includes(space)) this.refuse(name, t("surface.why.otherSpace"));
      params.space = space;
      return params;
    }
    const sessionId = params.session_id;
    if (typeof sessionId !== "string") throw new Invalid("`session_id` names a session of the application's own space");
    const instance = typeof params.instance === "string" ? params.instance : SYSTEM_INSTANCE;
    if (!(await this.ownsSession(instance, sessionId))) this.refuse(name, t("surface.why.otherSpace"));
    return params;
  }

  /** Whether the session is in one of the application's own spaces. */
  private async ownsSession(instance: string, sessionId: string): Promise<boolean> {
    for (const space of this.host.iface.spaces) {
      const { sessions } = await this.host.app.call<{ sessions: { session_id: string }[] }>("harnix.sessions", { space, instance });
      if (sessions.some((s) => s.session_id === sessionId)) return true;
    }
    return false;
  }

  /** The application's own space must be in front for what acts on the front space. */
  private ownFront(name: string): string {
    const space = this.host.space;
    if (!space || this.host.app.snapshot.space !== space) this.refuse(name, t("surface.why.notFront"));
    return space;
  }

  private async handle(method: string, p: Record<string, unknown>): Promise<unknown> {
    const { app, iface, decl } = this.host;
    const text = (name: string, max = MAX_TEXT) => {
      const v = p[name];
      if (typeof v !== "string") throw new Invalid(`\`${name}\` is a string`);
      return v.slice(0, max);
    };
    switch (method as keyof FrameRequests) {
      case "hello": {
        if (this.helloTimer) clearTimeout(this.helloTimer);
        this.helloTimer = null;
        this.start();
        this.host.ready();
        return {
          surface: {
            app: iface.id,
            point: decl.point,
            id: decl.id,
            space: decl.space,
            params: this.host.params ?? null,
            autoHeight: autoHeight(decl.point),
            links: iface.links,
          },
          env: this.env(),
          current: this.current(),
        } satisfies Welcome;
      }
      case "call": {
        const name = text("method", 200);
        if (!admits(iface.id, iface.hub, name, false)) this.refuse(name, t("surface.why.notListed"));
        return app.call(name, await this.narrow(name, FRAME_HUB[name], p.params ?? {}));
      }
      case "subscribe": {
        const topic = text("topic", 200);
        if (!admits(iface.id, iface.hub, topic, true)) this.refuse(topic, t("surface.why.notListed"));
        const params = (await this.narrow(topic, FRAME_HUB[`topic:${topic}`], p.params ?? {})) as Record<string, unknown>;
        if (this.disposed) return null;
        const sub = this.nextSub++;
        const off = topicsOf(app.client).subscribe(topic, params, (value) =>
          this.send({ kind: "event", name: "topic", data: { sub, value } satisfies ShellEvents["topic"] }),
        );
        this.subs.set(sub, off);
        return { sub };
      }
      case "unsubscribe": {
        const sub = Number(p.sub);
        this.subs.get(sub)?.();
        this.subs.delete(sub);
        return null;
      }
      case "session.current":
        return this.current();
      case "session.open": {
        const ref = { instance: text("instance", 200), sessionId: text("sessionId", 200) };
        const space = this.host.space;
        const backend = this.spaceSessions;
        if (!space || !backend) this.refuse("session.open", t("surface.why.noSpace"));
        const { sessions } = await app.call<{ sessions: { session_id: string; instance?: string }[] }>("harnix.sessions", { space: backend, instance: ref.instance });
        if (!sessions.some((s) => s.session_id === ref.sessionId)) this.refuse("session.open", t("surface.why.otherSpace"));
        if (app.snapshot.space !== space) await app.switchSpace(space);
        app.open(ref, space);
        return null;
      }
      case "session.create": {
        const space = this.host.space;
        const backend = this.spaceSessions;
        if (!space || !backend) this.refuse("session.create", t("surface.why.noSpace"));
        const spec: Record<string, unknown> = { space: backend };
        if (typeof p.scene === "string") spec.scene = p.scene;
        if (p.options && typeof p.options === "object") spec.options = p.options;
        const created = await app.call<{ session_id: string; instance?: string }>("session.create", spec);
        const ref = { instance: created.instance ?? SYSTEM_INSTANCE, sessionId: created.session_id };
        await app.refreshSessions();
        app.open(ref, space);
        return ref;
      }
      case "spaces.switch": {
        const space = text("space", 200);
        if (!app.registry.at("space").some((c) => c.id === space)) throw new Error(t("host.noSpace", { space }));
        await app.switchSpace(space);
        return null;
      }
      case "details.open": {
        const space = this.ownFront("details.open");
        app.toggleDetails(true);
        if (typeof p.tab === "string") {
          const contribution = app.registry.at("space").find((c) => c.id === space);
          const tabs = typeof contribution?.tabs === "function" ? contribution.tabs() : (contribution?.tabs ?? []);
          if (tabs.some((x) => x.id === p.tab)) app.selectTab(p.tab, true);
        }
        return null;
      }
      case "details.show": {
        const item = p.item as DetailItem | undefined;
        const kinds = iface.surfaces.filter((s) => s.point === "details.panel").flatMap((s) => s.kinds);
        if (!item || typeof item.kind !== "string") throw new Invalid("`item` is an object with a `kind`");
        if (!kinds.includes(item.kind)) this.refuse("details.show", t("surface.why.notKind", { kind: item.kind }));
        app.showDetail(JSON.parse(JSON.stringify(item)) as DetailItem);
        return null;
      }
      case "secondary.open":
        this.ownFront("secondary.open");
        if (!iface.surfaces.some((s) => s.point === "space.secondary" && s.space === this.host.space)) this.refuse("secondary.open", t("surface.why.noSurface"));
        this.host.secondary({ params: p.params ?? null, title: typeof p.title === "string" ? p.title.slice(0, 200) : undefined });
        return null;
      case "secondary.close":
        this.host.secondary(null);
        return null;
      case "settings.open": {
        const sections = iface.surfaces.filter((s) => s.point === "settings.section").map((s) => `${iface.id}.${s.id}`);
        const wanted = typeof p.section === "string" ? `${iface.id}.${p.section}` : sections[0];
        if (!wanted || !sections.includes(wanted)) this.refuse("settings.open", t("surface.why.noSurface"));
        openSettings(app, wanted);
        return null;
      }
      case "composer.insert":
        this.ownFront("composer.insert");
        app.insert(text("text"));
        return null;
      case "badge": {
        const v = p.value;
        this.host.badge(typeof v === "number" || (typeof v === "string" && v.length <= 8) ? v : null);
        return null;
      }
      case "notify": {
        const level = p.level === "error" ? "error" : "info";
        // Under the application's name: a line from a frame never reads as the shell's own.
        app.notice(t("surface.notice", { app: titleOf(iface), text: text("text", 300) }), level);
        return null;
      }
      case "copy":
        await navigator.clipboard.writeText(text("text"));
        return null;
      case "openModal": {
        const id = text("id", 64);
        if (!iface.surfaces.some((s) => s.point === "modal" && s.id === id)) this.refuse("openModal", t("surface.why.noSurface"));
        return this.host.openModal(id, p.params ?? null);
      }
      case "confirm":
        return ask({
          app: titleOf(iface),
          kind: "confirm",
          text: text("text", 2000),
          title: typeof p.title === "string" ? p.title.slice(0, 120) : undefined,
          confirm: typeof p.confirm === "string" ? p.confirm.slice(0, 40) : undefined,
          danger: Boolean(p.danger),
        });
      case "openLink": {
        const raw = text("url", 2000);
        if (!iface.links) this.refuse("openLink", t("surface.why.noLinks"));
        let url: URL;
        try {
          url = new URL(raw);
        } catch {
          throw new Invalid("`url` is an absolute URL");
        }
        if (url.protocol !== "https:" && url.protocol !== "http:") this.refuse("openLink", t("surface.why.notWeb"));
        if (!(await ask({ app: titleOf(iface), kind: "link", text: url.href }))) return false;
        window.open(url.href, "_blank", "noopener,noreferrer");
        return true;
      }
      case "upload": {
        const data = p.data;
        if (!(data instanceof ArrayBuffer)) throw new Invalid("`data` is the file's bytes");
        const file = new File([data], text("name", 255), { type: typeof p.type === "string" ? p.type : "" });
        return { path: await app.upload(file) };
      }
      default:
        throw new Unknown(t("surface.unknownRequest", { method }));
    }
  }

  private onEvent(name: string, data: unknown): void {
    const d = (data ?? {}) as Record<string, unknown>;
    if (name === "resize" && typeof d.height === "number" && Number.isFinite(d.height)) this.host.resize(d.height);
    else if (name === "close" && this.host.decl.point === "modal") this.host.close(d.result ?? null);
    else if (name === "key") this.key(d as unknown as KeyPress);
    else if (name === "error" && typeof d.message === "string") {
      this.host.app.registry.note({ point: "surface", owner: this.host.iface.id, id: this.host.decl.id, reason: d.message.slice(0, 500) });
    }
  }

  /**
   * A key the frame handed over: the shell acts on it as if it were pressed on the page — its own
   * listeners see it, and nothing in the frame does. Escape only from a frame inside a dialog
   * (closing that dialog); the shortcuts from any.
   */
  private key(press: KeyPress): void {
    const which = shellKey(press);
    if (!which || (which === "escape" && !inDialog(this.host.decl.point))) return;
    window.dispatchEvent(
      new KeyboardEvent("keydown", { key: press.key, code: press.code, metaKey: press.meta, ctrlKey: press.ctrl, shiftKey: press.shift, altKey: press.alt, bubbles: true, cancelable: true }),
    );
  }

  /** After hello: the heartbeat, and following the environment and the space's session. */
  private start(): void {
    this.beat = setInterval(() => {
      if (document.visibilityState !== "visible") return;
      if (this.waitingPong && ++this.misses >= MISSES) {
        this.fail(t("surface.failed.unresponsive"));
        return;
      }
      this.waitingPong = true;
      this.send({ kind: "request", id: this.nextRequest++, method: "ping", params: {} });
    }, PING_MS);
    this.offs.push(onShellEnvironment(() => this.pushEnv()));
    this.lastSession = JSON.stringify(this.current());
    this.offs.push(
      this.host.app.subscribe(() => {
        const now = JSON.stringify(this.current());
        if (now === this.lastSession) return;
        this.lastSession = now;
        this.send({ kind: "event", name: "session", data: { current: this.current() } satisfies ShellEvents["session"] });
      }),
    );
    this.lastEnv = JSON.stringify(this.env());
  }

  private env(): Environment {
    return { ...shellEnvironment(), visible: this.local.visible && document.visibilityState === "visible", modal: this.local.modal };
  }

  private pushEnv(): void {
    if (!this.port || this.helloTimer) return;
    const env = this.env();
    const key = JSON.stringify(env);
    if (key === this.lastEnv) return;
    this.lastEnv = key;
    this.send({ kind: "event", name: "env", data: env });
  }
}
