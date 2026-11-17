/**
 * `useHost()` — what a UI package, a page module and a contribution may do (concept §5.6). The
 * shell provides it; a contribution's component calls it.
 *
 * Deliberately not here: the connection. A contribution calls methods and subscribes to topics by
 * name and never learns which topology carries them, so the same code runs over one duplex
 * connection or split streams. Calls are limited to what `harnix.reachable` lists for this page,
 * so a contribution cannot reach a method the person using it could not.
 *
 * Sessions are always `(instance, session)`.
 */

import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import type { ExtensionEntry, Methods, Notifications, ParsedSessionEvent, SessionRow } from "@harnix/core";
import type { SessionState } from "@harnix/core/state";

import type { DetailItem } from "./contrib";
import type { Translate } from "./i18n/index";
import type { SidebarOpenOptions, SidebarResourceOptions } from "./sidebar";

export interface SessionRef {
  instance: string;
  sessionId: string;
}

/** The centre's own view of a session: what `host.conversation.current()` answers when no view is chosen. */
export const CONVERSATION_VIEW = "conversation";

/** One line under the left column's brand that the application owns (`host.shell.context`,
 *  ipdrun-49): what it is working on right now. */
export interface ShellContext {
  label: string;
  /** The hover tooltip. */
  title?: string;
  /** `active`: a small dot in front; `muted`: quieter text. Neither: plain. */
  tone?: "active" | "muted";
  /** Given: the line is a button that calls it. */
  onClick?: () => void;
}

/** A turn ended (`harnix.turn_settled`), done, failed or interrupted. */
export type TurnSettled = Notifications["harnix.turn_settled"];
/** Something about the host, not one session (`harnix.host_event`). */
export type HostEvent = Notifications["harnix.host_event"];

/** One event of a session this page is attached to. */
export interface SessionEventView {
  /** The engine's shape: not stable, it follows `hello.engine_protocol`. */
  raw: unknown;
  /** `@harnix/core`'s reading of it; `{ kind: "other" }` for what it does not know. */
  parsed: ParsedSessionEvent;
  /** `extension`: a session of an engine in an extension's process — draw its content untrusted. */
  trust: "product" | "extension";
}

/** One item of `host.dialogs.menu`. */
export interface MenuItem {
  id: string;
  label: string;
  icon?: string;
  danger?: boolean;
  disabled?: boolean;
  run(): void | Promise<void>;
}

export interface HostApi {
  /** One call, if `harnix.reachable` lists it; refused here otherwise. */
  call<T = unknown>(method: string, params?: unknown): Promise<T>;
  /** A host topic's whole values, now and on every change. Returns the unsubscribe. */
  subscribe(topic: string, params: Record<string, unknown> | undefined, fn: (value: unknown) => void): () => void;
  session: {
    /** The open session, if any. */
    current(): SessionRef | null;
    /** Its state: blocks, queue, running turn. */
    context(): SessionState | null;
    list(): SessionRow[];
    /**
     * Open a session in the front space. With `switchSpace`, in the space it belongs to (as the hub's
     * ledger has it), brought to the front first; the promise settles once it is open, and rejects
     * when there is no such session. Without options, exactly as before: the front space.
     *
     * With `reveal`, once open and its transcript caught up, the entry with that key is scrolled to
     * the centre and highlighted (see `reveal`); the promise says whether it was found.
     */
    open(ref: SessionRef, options: { switchSpace?: boolean; reveal: string; view?: string }): Promise<boolean>;
    open(ref: SessionRef, options?: { switchSpace?: boolean; reveal?: undefined; view?: string }): void | Promise<void>;
    /**
     * In the open session's conversation, the entry with this key — an element with
     * `data-reveal="<key>"` (a `flow.block` renderer puts it on its block), else the block whose id is
     * the key — scrolled to the centre and highlighted for a moment (`data-revealed`). Waits for the
     * transcript to catch up, within a bound; false when it is not found (or no session is open).
     */
    reveal(key: string): Promise<boolean>;
    create(spec?: { scene?: string; instance?: string; project_root?: string | null; options?: Record<string, unknown> }): Promise<SessionRef>;
    /** The front space's current session changed (opened, created, closed, deleted, another space in
     *  front). Not called again for the same session. Returns the unsubscribe. */
    onChange(fn: (current: SessionRef | null) => void): () => void;
    /** Whether a space on this page has the session open: a page that attached a session only to
     *  look at it leaves it attached when it stops, rather than taking it from the conversation. */
    isOpen(ref: SessionRef): boolean;
    /**
     * The shell's menu for a session — its own items (open, rename, fork, close, delete) and every
     * `session.action` at `menu` — anchored at an element or a point. For a package that lists
     * sessions in its own column: its rows open the same menu the title bar does. `name` is what
     * the menu shows and renaming starts from.
     */
    menu(ref: SessionRef & { name?: string | null }, anchor: Element | { x: number; y: number }): void;
  };
  spaces: {
    /** Bring this space to the front, with its own current session; settles once it is drawn.
     *  Rejects when there is no such space. */
    switch(id: string): Promise<void>;
    /** The front space's id. */
    current(): string;
  };
  /**
   * Which view of the centre the open session is shown in (harnix-ext.md §22.10): the
   * conversation (`CONVERSATION_VIEW`), or a `conversation.view` registration's id. Each session
   * remembers its own choice in this browser; `session.open(ref, { view })` opens with one.
   */
  conversation: {
    /** The open session's view: `CONVERSATION_VIEW` while it is the conversation, or no session is open. */
    current(): string;
    /** Show the open session in this view; an id no registration has falls back to the conversation. */
    show(view: string): void;
    /** The open session's view changed — switched, or another session opened. Returns the unsubscribe. */
    onChange(fn: (view: string) => void): () => void;
  };
  details: {
    /** Open the right column; with a tab, select it (one of the front space's tabs, or "details"). */
    open(tab?: string): void;
    close(): void;
    isOpen(): boolean;
    /** The selected tab's id. */
    tab(): string;
    /** The selected tab changed — clicked, or by `open(tab)`. Returns the unsubscribe. */
    onTab(fn: (tab: string) => void): () => void;
  };
  /**
   * The right column as the open session's docking surface (harnix-ext.md §22.7): every way into
   * it goes through here. Opening expands the column; the same content already open is focused,
   * not opened twice. An address no type claims, or a kind no type has, throws — a wiring error.
   */
  sidebar: {
    /** Open `harnix-resource://<type>/…` with the type that claims it (or `kind`'s); answers the tab's id. */
    openResource(address: string, options?: SidebarResourceOptions): string;
    /** Open a page type by kind; answers the tab's id. */
    openTab(kind: string, options?: SidebarOpenOptions): string;
    /** Close a tab of the open session's surface; nothing for an id that is not there. */
    close(tabId: string): void;
    /** The open session's tab in front, if it has one. */
    active(): { id: string; kind: string; address: string | null } | null;
    isExpanded(): boolean;
    toggleExpanded(open?: boolean): void;
  };
  settings: {
    /** Open the settings panel at a section (a `settings.section` id, or a built-in one such as
     *  "models"); the first section when there is no such one. */
    open(section?: string): void;
  };
  /** The front space's functions (tabs, title, badge, notice…) changed: the shell reads them again. */
  redraw(): void;
  /** The instances, typed as the protocol answers them (`Methods` in `@harnix/core`). */
  instances: {
    list(): Promise<Methods["harnix.instances.list"]["result"]>;
    sessions(instance: string): Promise<Methods["harnix.sessions.list"]["result"]>;
    tasks(instance: string, sessionId: string): Promise<Methods["harnix.session.tasks"]["result"]>;
    create(template: string, name: string, params?: Record<string, string>): Promise<Methods["harnix.instances.create"]["result"]>;
    stop(instance: string): Promise<Methods["harnix.instances.stop"]["result"]>;
  };
  composer: {
    /**
     * Put text into the open session's input, at the caret. With `session`, into that session's
     * input: at once if it is on the page, else kept for it (added after text already kept) and put
     * in when it is — so it may come before or after `session.open(session)`.
     */
    insert(text: string, options?: { session?: SessionRef }): void;
  };
  shell: {
    /** The page title's count prefix: n > 0 ⇒ `(n) <title>`; null or 0 ⇒ none. The shell puts it on
     *  whenever it writes the title (brand name, space), so an application never touches
     *  `document.title` (ipdrun-33). */
    badge(n: number | null): void;
    /** One line under the brand in the left column, the same in every space; null (or an empty
     *  label) ⇒ none. Each call replaces the last (ipdrun-49). */
    context(c: ShellContext | null): void;
    /** A line in the shell's banner: an error stays longer. The same line already waiting is not
     *  queued twice. */
    notice(text: string, tone?: "info" | "error"): void;
  };
  modal: {
    /** The shell's own modal layer (settings, dialogs) is open. */
    isOpen(): boolean;
    /** It opened or closed. Returns the unsubscribe. While it is open, Escape closes it and goes no
     *  further: document- and window-level key listeners do not see it. */
    onChange(fn: (open: boolean) => void): () => void;
  };
  /**
   * An extension's own link (concept §3.3.5): its routes, same-origin, with the credential put on
   * by the host. Paths cannot leave the extension's prefix. These links do not get the guarantees
   * of `call` and `subscribe` — topology independence, catching up by `seq`.
   */
  ext(id: string): {
    fetch(path: string, init?: RequestInit): Promise<Response>;
    socket(path: string): WebSocket;
  };
  /** Open something in the details pane: a card in a message reveals what it is about. */
  reveal(item: DetailItem): void;
  t: Translate;
  events: {
    /** A turn ended, in any session this page knows. Returns the unsubscribe. */
    onTurnSettled(fn: (e: TurnSettled) => void): () => void;
    /** The events of a session this page is attached to. Returns the unsubscribe. */
    onSessionEvent(ref: SessionRef, fn: (e: SessionEventView) => void): () => void;
    onHostEvent(fn: (e: HostEvent) => void): () => void;
  };
  /** Across spaces: bring `space` to the front, open `session` there, and hand `selection` to the
   *  space's `accept`. Rejects when there is no such space. */
  navigate: { open(target: { space: string; session?: SessionRef; selection?: unknown }): Promise<void> };
  /** The shell's own layer (focus, Escape, the page inert under it): an extension makes none. */
  dialogs: {
    confirm(text: string, opts?: { title?: string; confirm?: string; danger?: boolean }): Promise<boolean>;
    prompt(opts: { title: string; label: string; value?: string; validate?(v: string): string | null }): Promise<string | null>;
    /** A dialog of one's own: `render` lays out its body and buttons (`DialogBody`, `DialogFoot`). */
    open<T>(render: (close: (result: T | null) => void) => ReactNode, opts?: { title?: string; wide?: boolean }): Promise<T | null>;
    menu(anchor: Element | { x: number; y: number }, items: MenuItem[]): Promise<void>;
  };
  /** For diagnostics and an about page — not for deciding whether to fill another's point: that
   *  is whether the point is defined. */
  extensions: { list(): ExtensionEntry[]; onChange(fn: () => void): () => void };
  /** `hello.defaults`: neither the shell nor an extension writes "main" or "sessions" in. */
  defaults: { instance: string; space: string | null };
  /** The directory this product keeps in each project (`hello.shell.brand.project_dir`): `.harnix`,
   *  or a branded application's `.<product_id>`. Absent from a host that does not know it. */
  projectDir?: string;
}

const HostContext = createContext<HostApi | null>(null);

export function HostProvider({ host, children }: { host: HostApi; children: ReactNode }) {
  return <HostContext.Provider value={host}>{children}</HostContext.Provider>;
}

export function useHost(): HostApi {
  const host = useContext(HostContext);
  if (!host) throw new Error("@harnix/react: useHost() needs a <HostProvider> above it");
  return host;
}

/** The host, or null where there is none (the conversation drawn on its own, outside the shell). */
export function useOptionalHost(): HostApi | null {
  return useContext(HostContext);
}

/** The front space's current session, kept up to date: `session.current()` and `session.onChange`. */
export function useCurrentSession(): SessionRef | null {
  const host = useHost();
  const [current, setCurrent] = useState<SessionRef | null>(() => host.session.current());
  useEffect(() => {
    setCurrent(host.session.current());
    return host.session.onChange(setCurrent);
  }, [host]);
  return current;
}
