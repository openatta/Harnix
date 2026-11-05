/**
 * The module's topics and calls, as React state. Topics are counted by the host, so two parts of
 * the page reading one run share one subscription.
 */

import { useCallback, useEffect, useState } from "react";
import { useNest } from "@harnix/react";

import { methodName, runTopic } from "./model";
import type { FlowMethod, RunView } from "./types";

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

/** One run, live. */
export function useRunView(module: string, run: string | null): RunView | undefined {
  return useTopic<RunView>(runTopic(module), run ? { run } : null);
}

export interface Loaded<T> {
  value: T | undefined;
  error: string | null;
  reload(): void;
}

/** A module method's answer for these params, called again when they change; null params: not called. */
export function useMethod<T>(module: string, name: FlowMethod, params: Record<string, unknown> | null): Loaded<T> {
  const nest = useNest();
  const [value, setValue] = useState<T | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  const [nonce, setNonce] = useState(0);
  const key = params ? JSON.stringify(params) : null;
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
  }, [nest, module, name, key, nonce]);
  const reload = useCallback(() => setNonce((n) => n + 1), []);
  return { value, error, reload };
}

export function errorText(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (e && typeof e === "object" && typeof (e as { message?: unknown }).message === "string") return (e as { message: string }).message;
  return String(e);
}
