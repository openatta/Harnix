/**
 * `useNest()` — what a contribution may do (design §10.4). The interface provides it; a
 * contribution's component calls it.
 *
 * Deliberately not here: the connection. A contribution calls methods and subscribes to topics by
 * name and never learns which topology carries them, so the same code runs over one duplex
 * connection or split streams. Calls are limited to what `harnix.reachable` lists for this page,
 * so a contribution cannot reach a method the person using it could not.
 *
 * Sessions are always `(instance, session)`.
 */

import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import type { Methods, SessionRow } from "@harnix/core";
import type { SessionState } from "@harnix/core/state";

import type { DetailItem } from "./contrib";
import type { Translate } from "./i18n/index";

export interface SessionRef {
  instance: string;
  sessionId: string;
}

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

export interface NestHost {
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
    open(ref: SessionRef, options: { switchSpace?: boolean; reveal: string }): Promise<boolean>;
    open(ref: SessionRef, options?: { switchSpace?: boolean; reveal?: undefined }): void | Promise<void>;
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
  };
  spaces: {
    /** Bring this space to the front, with its own current session; settles once it is drawn.
     *  Rejects when there is no such space. */
    switch(id: string): Promise<void>;
    /** The front space's id. */
    current(): string;
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
    sessions(instance: string): Promise<Methods["harnix.instances.sessions"]["result"]>;
    tasks(instance: string, sessionId: string): Promise<Methods["harnix.sessions.tasks"]["result"]>;
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
  };
  modal: {
    /** The shell's own modal layer (settings, dialogs) is open. */
    isOpen(): boolean;
    /** It opened or closed. Returns the unsubscribe. While it is open, Escape closes it and goes no
     *  further: document- and window-level key listeners do not see it. */
    onChange(fn: (open: boolean) => void): () => void;
  };
  /**
   * A module's own link (design §7.7): its routes under `/apps/<module>/`, same-origin, with the
   * credential put on by the host. Paths cannot leave the module's prefix. These links do not get
   * the guarantees of `call` and `subscribe` — topology independence, catching up by `seq`.
   */
  space(module: string): {
    fetch(path: string, init?: RequestInit): Promise<Response>;
    socket(path: string): WebSocket;
  };
  /** Open something in the details pane: a card in a message reveals what it is about. */
  reveal(item: DetailItem): void;
  t: Translate;
}

const NestContext = createContext<NestHost | null>(null);

export function NestProvider({ host, children }: { host: NestHost; children: ReactNode }) {
  return <NestContext.Provider value={host}>{children}</NestContext.Provider>;
}

export function useNest(): NestHost {
  const host = useContext(NestContext);
  if (!host) throw new Error("@harnix/react: useNest() needs a <NestProvider> above it");
  return host;
}

/** The front space's current session, kept up to date: `session.current()` and `session.onChange`. */
export function useCurrentSession(): SessionRef | null {
  const host = useNest();
  const [current, setCurrent] = useState<SessionRef | null>(() => host.session.current());
  useEffect(() => {
    setCurrent(host.session.current());
    return host.session.onChange(setCurrent);
  }, [host]);
  return current;
}
