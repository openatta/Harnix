/**
 * A host for the system space's tests: calls answered by `answer` and recorded, topics pushed by
 * `push`, the details column and the space in front kept as plain fields.
 */

import type { HostApi } from "@harnix/react";

type Topic = { topic: string; params: Record<string, unknown>; fn: (value: unknown) => void };

export interface FakeHost {
  host: HostApi;
  calls: { method: string; params: unknown }[];
  /** Push a value to every subscriber of `topic` (whatever its params). */
  push(topic: string, value: unknown): void;
  answer: (method: string, params: unknown) => unknown;
  /** The tab `details.open` was last asked for, and the space switched to. */
  shown: { tab?: string; space?: string; notices: string[] };
}

export function fakeHost(): FakeHost {
  const topics = new Set<Topic>();
  const fake: FakeHost = {
    calls: [],
    answer: () => ({}),
    shown: { notices: [] },
    push(topic, value) {
      for (const t of [...topics]) if (t.topic === topic) t.fn(value);
    },
    host: undefined as unknown as HostApi,
  };
  const none = () => () => undefined;
  const host = {
    call: async (method: string, params?: unknown) => {
      fake.calls.push({ method, params });
      return fake.answer(method, params);
    },
    subscribe(topic: string, params: Record<string, unknown> | undefined, fn: (value: unknown) => void) {
      const entry = { topic, params: params ?? {}, fn };
      topics.add(entry);
      return () => topics.delete(entry);
    },
    session: { current: () => null, isOpen: () => false, onChange: none, menu: () => undefined },
    spaces: {
      switch: async (id: string) => {
        fake.shown.space = id;
      },
      current: () => fake.shown.space ?? "",
    },
    details: {
      open: (tab?: string) => {
        if (tab !== undefined) fake.shown.tab = tab;
      },
      close: () => undefined,
      isOpen: () => true,
      tab: () => fake.shown.tab ?? "",
      onTab: none,
    },
    redraw: () => undefined,
    shell: { badge: () => undefined, context: () => undefined, notice: (text: string) => void fake.shown.notices.push(text) },
    events: { onTurnSettled: none, onSessionEvent: none, onHostEvent: none },
    navigate: { open: async () => undefined },
    dialogs: { confirm: async () => true, prompt: async () => null, open: async () => null, menu: async () => undefined },
    defaults: { instance: "main", space: "sessions" },
    t: (key: string) => key,
  };
  fake.host = host as unknown as HostApi;
  return fake;
}
