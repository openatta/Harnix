/**
 * Where things are, and what this machine can reach.
 */

import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/** The Harnix checkout: four levels above this file. */
export const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..", "..");

/** Recorded model calls, replayed by position (design §13.3). */
export const RECORDINGS = join(ROOT, "tests", "live", "fixtures", "recordings");

/**
 * Read `.env` the way a shell would.
 *
 * Tests that need a model are skipped without one rather than failed: a
 * missing credential is a fact about the machine, and turning it into a red
 * test trains people to ignore red tests.
 */
export function loadEnv(): Record<string, string> {
  const path = join(ROOT, ".env");
  if (!existsSync(path)) return {};
  const env: Record<string, string> = {};
  for (const line of readFileSync(path, "utf8").split("\n")) {
    const match = line.match(/^\s*(?:export\s+)?([A-Z_][A-Z0-9_]*)\s*=\s*(.*)\s*$/);
    if (!match) continue;
    env[match[1]!] = match[2]!.replace(/^["']|["']$/g, "");
  }
  return env;
}

export function hasModel(env: Record<string, string>): boolean {
  return Boolean(env.ANTHROPIC_AUTH_TOKEN || env.ANTHROPIC_API_KEY);
}

/**
 * How this run reaches a model.
 *
 * - `replay` (the default): the engine plays recorded calls back. Deterministic,
 *   offline, free.
 * - `live`: the provider in `.env`. For recording fixtures and smoke runs —
 *   `pnpm live --live`, or `HARNIX_MODEL=live`.
 * - `none`: nothing model-shaped runs.
 */
export type ModelMode = "replay" | "live" | "none";

export function modelMode(): ModelMode {
  const asked = process.env.HARNIX_MODEL;
  return asked === "live" || asked === "none" ? asked : "replay";
}

export const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
