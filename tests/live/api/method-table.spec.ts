/**
 * The authorization table, pinned (concept §3.4): every name a client may call, subscribe to or
 * fetch, who owns it and who may reach it — as `harnix.diagnostics` reports it, against
 * `crates/harnix/tests/method-table.snap.json`. A refactor that moves a row, loses one or opens
 * one fails here before it fails anywhere a user would see.
 *
 * `harnix.reachable` is the same table asked by subject: a device on the Harnix app, a
 * compiled-in module (`probe`, on the test application), and a runtime application's subject
 * (its half is the Rust test in `crates/app/src/gate.rs`, which reads the same file).
 */

import { type Backend, type TestClient, connect, expect, test } from "@harnix/testkit";

import { matchSnapshot, matchSnapshotKey } from "./snapshot";
import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:authz", "@layer:api"] };

const names = (list: string[]): string[] => [...list].sort();

test("the authorization table is the snapshot's", TAGS, async ({ api }) => {
  const { table } = await api.call("harnix.diagnostics");
  matchSnapshot("method-table.snap.json", table);
});

test("every row says who owns it and how far it is open, and the device's reach is what the rows say", TAGS, async ({ api }) => {
  const { table } = await api.call("harnix.diagnostics");
  const rows: any[] = table.rows;
  expect(rows.length).toBeGreaterThan(50);
  for (const row of rows) {
    expect(row.owner, row.name).toMatch(/^(kernel:[a-z]+|ext:[a-z0-9.-]+|engine)$/);
    expect(Object.keys(row.access).sort(), row.name).toEqual(
      ["device", "engine", "extensions", "hub_owned", "refused", "requires", "scope"],
    );
    // Refused to everyone, grants included: nothing may reach what the hub owns.
    if (row.access.hub_owned) {
      expect(row.access.device, row.name).toBe(false);
      expect(row.access.extensions, row.name).toEqual([]);
    }
  }
  const reach = await api.call("harnix.reachable");
  const open = (kind: string) =>
    rows.filter((r) => r.kind === kind && r.access.device && !r.access.refused).map((r) => r.name);
  expect(names(reach.methods)).toEqual(names(open("method")));
  expect(names(reach.topics)).toEqual(names(open("topic")));
});

test("a device's reach is the snapshot's", TAGS, async ({ api }) => {
  const reach = await api.call("harnix.reachable");
  matchSnapshotKey("reachable.snap.json", "device", {
    methods: names(reach.methods),
    topics: names(reach.topics),
  });
});

test.describe("a compiled-in module's reach", () => {
  let backend: Backend;
  let api: TestClient;

  test.beforeAll(async () => {
    backend = await startTestApp();
    api = await connect(backend);
  });
  test.afterAll(async () => {
    api?.close();
    await backend?.dispose();
  });

  test("is the snapshot's", TAGS, async () => {
    // `app.probe.call` (crates/testapp) calls a method as the `probe` module.
    const reach = await api.call("app.probe.call", { method: "harnix.reachable", params: {} });
    expect(reach.subject).toEqual({ kind: "module", id: "probe" });
    matchSnapshotKey("reachable.snap.json", "module", {
      methods: names(reach.methods),
      topics: names(reach.topics),
    });
  });
});
