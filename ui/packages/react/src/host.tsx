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

import { createContext, useContext, type ReactNode } from "react";
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
    open(ref: SessionRef): void;
    create(spec?: { scene?: string; instance?: string; project_root?: string | null; options?: Record<string, unknown> }): Promise<SessionRef>;
  };
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
