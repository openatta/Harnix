/** The shell's state, re-rendering on every change. */

import { useSyncExternalStore } from "react";

import type { App, ShellState } from "./app.ts";

export function useShell(app: App): ShellState {
  return useSyncExternalStore(app.subscribe, app.getSnapshot);
}
