/**
 * A client that is the server, for tests of the shell's app and host: it answers calls with
 * `answer` and records them. (`host.test.ts` keeps its own copy.)
 */

import type { HarnixClient } from "@harnix/core";
import { Registry, type SpaceContribution } from "@harnix/react";

import { App } from "../src/app";

type Listener = (method: string, params: Record<string, unknown>) => void;

export class ScriptedClient {
  status: "open" | "closed" | "connecting" = "open";
  calls: { method: string; params: Record<string, unknown> }[] = [];
  private notify = new Set<Listener>();
  private statusListeners = new Set<(s: string) => void>();
  answer: (method: string, params: Record<string, unknown>) => unknown = () => ({});

  call(method: string, params: Record<string, unknown> = {}): Promise<unknown> {
    this.calls.push({ method, params });
    try {
      return Promise.resolve(this.answer(method, params));
    } catch (e) {
      return Promise.reject(e);
    }
  }
  onNotify(fn: Listener): () => void {
    this.notify.add(fn);
    return () => this.notify.delete(fn);
  }
  onStatus(fn: (s: string) => void): () => void {
    this.statusListeners.add(fn);
    return () => this.statusListeners.delete(fn);
  }
}

export function space(id: string, extra: Partial<SpaceContribution> = {}): SpaceContribution {
  return { id, icon: "chat", title: () => id, ...extra };
}

/** An app over a scripted client, with the spaces "sessions" and "b". */
export function scriptedApp(): { app: App; client: ScriptedClient } {
  localStorage.clear();
  const client = new ScriptedClient();
  const app = new App(client as unknown as HarnixClient, new Registry());
  app.registry.register("space", space("sessions"), "test");
  app.registry.register("space", space("b"), "test");
  return { app, client };
}

export const tick = () => new Promise((r) => setTimeout(r, 0));
