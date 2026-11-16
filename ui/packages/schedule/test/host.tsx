/** A host for the tests: calls answered by method and recorded, the topic pushed by hand, dialogs and notices kept. */

import type { ReactNode } from "react";
import { render } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider, HostProvider, en, type HostApi } from "@harnix/react";

export interface Fake {
  host: HostApi;
  calls: [string, unknown][];
  dialogs: ((close: (r: unknown) => void) => ReactNode)[];
  notices: [string, string | undefined][];
  opened: [unknown, unknown][];
  confirms: string[];
  push(topic: string, value: unknown): void;
}

export function fakeHost(answer: (method: string, params: unknown) => unknown = () => ({}), options: { sessions?: unknown[]; front?: string } = {}): Fake {
  const calls: Fake["calls"] = [];
  const dialogs: Fake["dialogs"] = [];
  const notices: Fake["notices"] = [];
  const opened: Fake["opened"] = [];
  const confirms: string[] = [];
  const topics = new Map<string, Set<(v: unknown) => void>>();
  const values = new Map<string, unknown>();
  const fake: Fake = {
    calls,
    dialogs,
    notices,
    opened,
    confirms,
    push(topic, value) {
      values.set(topic, value);
      for (const fn of [...(topics.get(topic) ?? [])]) fn(value);
    },
    host: {
      call: async (method: string, params?: unknown) => {
        calls.push([method, params]);
        const value = answer(method, params);
        if (value instanceof Error || (value && typeof value === "object" && "code" in value && "message" in value)) throw value;
        return value;
      },
      subscribe: (topic: string, _params: unknown, fn: (v: unknown) => void) => {
        const set = topics.get(topic) ?? new Set();
        set.add(fn);
        topics.set(topic, set);
        if (values.has(topic)) fn(values.get(topic));
        return () => set.delete(fn);
      },
      session: {
        list: () => options.sessions ?? [],
        current: () => null,
        onChange: () => () => undefined,
        open: async (ref: unknown, opts?: unknown) => void opened.push([ref, opts]),
      },
      spaces: { current: () => options.front ?? "sessions" },
      dialogs: {
        open: async (render: (close: (r: unknown) => void) => ReactNode) => void dialogs.push(render),
        confirm: async (text: string) => (confirms.push(text), true),
      },
      shell: { notice: (text: string, tone?: string) => void notices.push([text, tone]) },
      defaults: { instance: "main", space: "sessions" },
    } as unknown as HostApi,
  };
  return fake;
}

const HELLO = { scenes: [{ scene: "harnix", name: "Harnix", active: true }, { scene: "code", name: "Code", active: true }] };

const client = {
  status: "open",
  detail: undefined,
  onStatus: () => () => undefined,
  call: async (method: string) => (method === "harnix.hello" ? HELLO : {}),
} as unknown as HarnixClient;

/** Drawn as the shell draws a dialog or a page: with the host, and a client for `hello`. */
export function drawn(fake: Fake, node: ReactNode) {
  return render(
    <HarnixUiProvider client={client} strings={en}>
      <HostProvider host={fake.host}>{node}</HostProvider>
    </HarnixUiProvider>,
  );
}
