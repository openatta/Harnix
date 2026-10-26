/**
 * Migrations (design §7.3, D29): Harnix decides when and what happens on
 * failure; the module decides how. In order, before `start`; a failure
 * disables that module alone.
 */

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { connect, expect, test } from "@harnix/testkit";

import { moduleStatus, startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:migrations", "@layer:api"] };

test("a module's migration runs before it starts, and only once per step", TAGS, async () => {
  const backend = await startTestApp();
  try {
    const api = await connect(backend);
    const probe = await moduleStatus(api, "probe");
    expect(probe.migrations).toEqual(["001-create", "002-index"]);
    const marker = join(backend.scratch, "projects", ".harnix", "modules", "probe", "schema");
    expect(existsSync(marker)).toBe(true);
    expect(readFileSync(marker, "utf8").split("\n")).toEqual(["001-create", "002-index"]);
    api.close();
  } finally {
    backend.stop();
  }
});

test("a failed migration disables that module alone", TAGS, async () => {
  const backend = await startTestApp();
  try {
    const api = await connect(backend);
    const failed = await moduleStatus(api, "fails-migrate");
    expect(failed.enabled).toBe(false);
    expect(failed.phase).toBe("migration");
    expect(failed.reason).toContain("on purpose");
    expect(await api.refused("app.fails-migrate.ping", {})).toBeTruthy();
    // The modules after it in the list started.
    expect((await moduleStatus(api, "probe")).enabled).toBe(true);
    expect((await api.call("app.probe.echo", { text: "up" })).text).toBe("up");
    api.close();
  } finally {
    backend.stop();
  }
});
