/**
 * The interface's `useNest()` host (design §10.4), built from the app: calls limited to what this
 * page may reach, topics by name, the open session, the composer, the details pane.
 */

import type { HarnixClient } from "@harnix/core";
import { SYSTEM_INSTANCE } from "@harnix/core/state";
import { t, type NestHost } from "@harnix/react";

import type { App } from "./app.ts";

export function hostFor(app: App, owner = "page"): NestHost {
  const call = <T,>(method: string, params?: unknown): Promise<T> => {
    const reachable = app.snapshot.reachable;
    if (reachable.length && !reachable.includes(method) && !method.startsWith("harnix.subscribe") && !method.startsWith("harnix.unsubscribe")) {
      return Promise.reject(new Error(t("contrib.methodNotDeclared", { owner, method })));
    }
    return app.call<T>(method, params);
  };
  return {
    call,
    subscribe(topic, params, fn) {
      return subscribe(app.client, topic, params ?? {}, fn);
    },
    session: {
      current: () => {
        const current = app.snapshot.current;
        return current ? { instance: current.instance, sessionId: current.sessionId } : null;
      },
      context: () => app.controller?.getSnapshot() ?? null,
      list: () => app.snapshot.sessions,
      open: (ref) => app.open(ref),
      async create(spec = {}) {
        await app.create(spec);
        const current = app.snapshot.current!;
        return { instance: current.instance ?? SYSTEM_INSTANCE, sessionId: current.sessionId };
      },
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
    reveal: (item) => app.showDetail(item),
    t,
  };
}

/** `harnix.subscribe`, then every `harnix.topic` for that topic and those params. */
function subscribe(client: HarnixClient, topic: string, params: Record<string, unknown>, fn: (value: unknown) => void): () => void {
  const same = (other: unknown) => JSON.stringify(other ?? {}) === JSON.stringify(params);
  let alive = true;
  const off = client.onNotify((method, frame) => {
    if (alive && method === "harnix.topic" && frame.topic === topic && same(frame.params)) fn(frame.value);
  });
  client
    .call("harnix.subscribe", { topic, params })
    .then((r) => alive && fn((r as { value: unknown }).value))
    .catch(() => undefined);
  return () => {
    alive = false;
    off();
    if (client.status === "open") client.call("harnix.unsubscribe", { topic, params }).catch(() => undefined);
  };
}
