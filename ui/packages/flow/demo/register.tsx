/**
 * The flow space as an interface module, for a shell build: `HARNIX_UI_MODULES` points here, and
 * the shell calls `register(registry, host)` once it is connected. Assumes the backend registered
 * the flow module under the id `flow` (`app.flow.*`).
 */

import type { NestHost, Registry } from "@harnix/react";

import { registerFlowSpace } from "../src/index";

export function register(registry: Registry, _host: NestHost): void {
  registerFlowSpace(registry);
}
