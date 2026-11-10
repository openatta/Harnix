/**
 * The snapshots under `crates/harnix/tests/`: the authorization table and what each kind of
 * subject reaches, as the process reports them. A change to either is a decision, so the file
 * changes in the same commit and the commit says why; `HARNIX_UPDATE_SNAPSHOTS=1` rewrites it.
 */

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { ROOT, expect } from "@harnix/testkit";

const text = (value: unknown): string => `${JSON.stringify(value, null, 2)}\n`;

function path(file: string): string {
  return join(ROOT, "crates", "harnix", "tests", file);
}

const updating = (): boolean => process.env.HARNIX_UPDATE_SNAPSHOTS === "1";

/** `actual` against the whole of `file`. */
export function matchSnapshot(file: string, actual: unknown): void {
  if (updating()) {
    writeFileSync(path(file), text(actual));
    return;
  }
  expect(existsSync(path(file)), `${file} is missing: HARNIX_UPDATE_SNAPSHOTS=1 writes it`).toBe(true);
  expect(actual, `${file} differs; if the change is intended: HARNIX_UPDATE_SNAPSHOTS=1`).toEqual(
    JSON.parse(readFileSync(path(file), "utf8")),
  );
}

/** `actual` against one key of `file`, which several tests and the Rust side share. */
export function matchSnapshotKey(file: string, key: string, actual: unknown): void {
  const kept = existsSync(path(file)) ? JSON.parse(readFileSync(path(file), "utf8")) : {};
  if (updating()) {
    const merged = { ...kept, [key]: actual };
    writeFileSync(path(file), text(Object.fromEntries(Object.keys(merged).sort().map((k) => [k, merged[k]]))));
    return;
  }
  expect(key in kept, `${file} has no \`${key}\`: HARNIX_UPDATE_SNAPSHOTS=1 writes it`).toBe(true);
  expect(actual, `${file}: \`${key}\` differs; if the change is intended: HARNIX_UPDATE_SNAPSHOTS=1`).toEqual(kept[key]);
}
