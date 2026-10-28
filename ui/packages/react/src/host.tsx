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
import type { SessionRow } from "@harnix/core";
import type { SessionState } from "@harnix/core/state";

import type { DetailItem } from "./contrib";
import type { Translate } from "./i18n/index";

export interface SessionRef {
  instance: string;
  sessionId: string;
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
     */
    open(ref: SessionRef, options?: { switchSpace?: boolean }): void | Promise<void>;
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
  instances: {
    list(): Promise<unknown>;
    sessions(instance: string): Promise<unknown>;
    tasks(instance: string, sessionId: string): Promise<unknown>;
    create(template: string, name: string, params?: Record<string, string>): Promise<unknown>;
    stop(instance: string): Promise<unknown>;
  };
  composer: {
    /** Put text into the open session's input, at the caret. */
    insert(text: string): void;
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
