/** A host for the tests: it answers calls by method, records them, and offers the few members the pages reach. */

import type { HostApi } from "@harnix/react";

export interface Scripted {
  host: HostApi;
  calls: { method: string; params: unknown }[];
  settle(sessionId: string): void;
  push(topic: string, value: unknown): void;
  /** What `host.conversation.show` was asked, and what `host.session.open` was asked. */
  shown: string[];
  opened: { instance: string; sessionId: string; options?: unknown }[];
  confirmed: boolean;
}

export function scripted(answer: (method: string, params: unknown) => unknown, options: { current?: { instance: string; sessionId: string } | null; space?: string } = {}): Scripted {
  const calls: Scripted["calls"] = [];
  const settled = new Set<(e: { session_id: string }) => void>();
  const topics = new Map<string, Set<(value: unknown) => void>>();
  const shown: string[] = [];
  const opened: Scripted["opened"] = [];
  const out: Scripted = {
    calls,
    shown,
    opened,
    confirmed: true,
    settle: (sessionId) => settled.forEach((fn) => fn({ session_id: sessionId })),
    push: (topic, value) => topics.get(topic)?.forEach((fn) => fn(value)),
    host: {
      call: async (method: string, params: unknown) => {
        calls.push({ method, params });
        const value = answer(method, params);
        if (value instanceof Error) throw value;
        return value;
      },
      subscribe: (topic: string, _params: unknown, fn: (value: unknown) => void) => {
        if (!topics.has(topic)) topics.set(topic, new Set());
        topics.get(topic)!.add(fn);
        return () => topics.get(topic)!.delete(fn);
      },
      events: {
        onTurnSettled: (fn: (e: { session_id: string }) => void) => {
          settled.add(fn);
          return () => settled.delete(fn);
        },
        onSessionEvent: () => () => undefined,
        onHostEvent: () => () => undefined,
      },
      defaults: { instance: "main", space: "sessions" },
      spaces: { current: () => options.space ?? "sessions", switch: async () => undefined },
      session: {
        current: () => options.current ?? null,
        open: async (ref: { instance: string; sessionId: string }, opts?: unknown) => {
          opened.push({ ...ref, options: opts });
          return true;
        },
      },
      conversation: { current: () => "conversation", show: (view: string) => shown.push(view), onChange: () => () => undefined },
      dialogs: { confirm: async () => out.confirmed },
      shell: { notice: () => undefined },
    } as unknown as HostApi,
  };
  return out;
}

/** A refusal as the host gives one for a method `harnix.reachable` does not list. */
export const NOT_DECLARED = new Error("method `x` is not declared for this page");

/** A recorded call, as `observability.trace` lists one. */
export function call(index: number, over: Partial<CallLike> = {}): CallLike {
  return {
    index,
    session_id: "s1",
    parent_session_id: null,
    agent_type: null,
    turn_id: "t1",
    turn: 0,
    step: index,
    purpose: null,
    ts: 1000 + index * 1000,
    provider: "anthropic",
    model: "m",
    input: { system: [], tools: 0, messages: 1 },
    output: { chars: 0, thinking_chars: 0, tool_uses: 0 },
    tools: [],
    status: "ok",
    error: null,
    stop_reason: "end_turn",
    usage: { input_tokens: 10, output_tokens: 5 },
    duration_ms: 500,
    timing: { sent_at: 1000 + index * 1000, ttft_ms: 100, generate_ms: 400, tokens_per_s: 12 },
    ...over,
  };
}

export interface CallLike {
  index: number;
  session_id: string;
  parent_session_id: string | null;
  agent_type: string | null;
  turn_id: string | null;
  turn: number;
  step: number;
  purpose: string | null;
  ts: number;
  provider: string;
  model: string;
  input: { system: { source: string | null; chars: number }[]; tools: number; messages: number };
  output: { chars: number; thinking_chars: number; tool_uses: number; text_preview?: string | null };
  tools: { id: string; name: string; input_preview?: string | null; result_is_error?: boolean | null }[];
  status: string;
  error: { kind?: string; message?: string } | null;
  stop_reason: string | null;
  usage: { input_tokens?: number; output_tokens?: number; cache_read_input_tokens?: number; cache_creation_input_tokens?: number } | null;
  duration_ms: number | null;
  timing: { sent_at: number; ttft_ms: number | null; generate_ms: number | null; tokens_per_s: number | null } | null;
}
