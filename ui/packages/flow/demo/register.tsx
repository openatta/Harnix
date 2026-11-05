/**
 * The flow space as an interface module, for an application that builds its own shell without the
 * built-in one (the Harnix shell registers it itself when the backend has the flow module, xops-02
 * §7): `HARNIX_UI_MODULES` points here, and the shell calls `register(registry, host)` once it is
 * connected. Assumes the backend registered the flow module under the id `flow` (`app.flow.*`) and
 * its space under `flow`.
 */

import type { NestHost, Registry } from "@harnix/react";

import { registerFlowSpace } from "../src/index";

export function register(registry: Registry, host: NestHost): void {
  registerFlowSpace(registry, { host });
}
