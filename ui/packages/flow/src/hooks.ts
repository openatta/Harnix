/**
 * The module's topics and calls, as React state. Topics are counted by the host, so two parts of
 * the page reading one run share one subscription.
 */

import { useCallback, useEffect, useState } from "react";
import { useNest } from "@harnix/react";

import { defsOf, defsTopic, dryRunsOf, dryRunsTopic, methodName, runTopic } from "./model";
import type { DefSummary, DryRunSummary, FlowMethod, RunView } from "./types";

/** A topic's latest value; undefined until the first arrives, or while `params` is null. */
export function useTopic<T>(topic: string, params: Record<string, unknown> | null): T | undefined {
  const nest = useNest();
  const [value, setValue] = useState<T | undefined>(undefined);
  const key = params ? JSON.stringify(params) : null;
  useEffect(() => {
    setValue(undefined);
    if (key === null) return undefined;
    return nest.subscribe(topic, JSON.parse(key) as Record<string, unknown>, (v) => setValue(v as T));
  }, [nest, topic, key]);
  return value;
}

/**
 * One run, live: the `run {run}` topic's value — and, until the topic has one (a run the module
 * has not published since it started: finished before a restart), what `get` answers.
 */
export function useRunView(module: string, run: string | null): RunView | undefined {
  const live = useTopic<RunView>(runTopic(module), run ? { run } : null);
  const fetched = useMethod<RunView>(module, "get", run && !live ? { run } : null);
  return live ?? fetched.value;
}

/** The flows, live: the `defs` topic, and the `defs` method until the topic has a value. */
export function useDefs(module: string): { list: DefSummary[]; loaded: boolean; error: string | null } {
  const live = useTopic<unknown>(defsTopic(module), {});
  const fetched = useMethod<unknown>(module, "defs", live === undefined ? {} : null);
  const value = live ?? fetched.value;
  return { list: defsOf(value), loaded: value !== undefined, error: live === undefined ? fetched.error : null };
}

/** The dry runs kept, live: the `dry_runs` topic, and `dry_run.list` until the topic has a value. */
export function useDryRuns(module: string): DryRunSummary[] {
  const live = useTopic<unknown>(dryRunsTopic(module), {});
  const fetched = useMethod<unknown>(module, "dry_run.list", live === undefined ? {} : null);
  return dryRunsOf(live ?? fetched.value);
}

export interface Loaded<T> {
  value: T | undefined;
  error: string | null;
  reload(): void;
}

/**
 * A module method's answer for these params, called again when they change — or when `again`
 * does (a digest, a revision: what says the answer is stale); null params: not called.
 */
export function useMethod<T>(module: string, name: FlowMethod, params: Record<string, unknown> | null, again?: unknown): Loaded<T> {
  const nest = useNest();
  const [value, setValue] = useState<T | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  const [nonce, setNonce] = useState(0);
  const key = params ? JSON.stringify(params) : null;
  const stale = again === undefined ? "" : JSON.stringify(again);
  useEffect(() => {
    if (key === null) {
      setValue(undefined);
      return undefined;
    }
    let live = true;
    setError(null);
    nest
      .call<T>(methodName(module, name), JSON.parse(key))
      .then((v) => live && setValue(v))
      .catch((e: unknown) => live && setError(errorText(e)));
    return () => {
      live = false;
    };
  }, [nest, module, name, key, nonce, stale]);
  const reload = useCallback(() => setNonce((n) => n + 1), []);
  return { value, error, reload };
}

export function errorText(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (e && typeof e === "object" && typeof (e as { message?: unknown }).message === "string") return (e as { message: string }).message;
  return String(e);
}
