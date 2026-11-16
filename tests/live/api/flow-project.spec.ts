/**
 * A project's own flows (crates/ext/flow/docs/guide.en.md, "A project's own flows";
 * docs/harnix-app.md §3.1): `<project>/.harnix/flows/`, committed with the project, offered to its
 * tasks over every other layer, and read only while the project is trusted.
 *
 * The project brings a flow of its own (`proj-check`, a confirmation alone: no session, no model),
 * one under a built-in flow's name (`demo-linear`), and a file that is not a definition. Trust is
 * the engine's, set as a person does: the project added in the projects space, then trusted there.
 */

import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { type Backend, type TestClient, connect, expect, test } from "@harnix/testkit";

import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:flow", "@layer:api"] };
const REFUSED = -31000;

const OWN = "name: proj-check\ntitle: The project's check\nstations: [{id: check, exit: [{kind: confirm}]}]\n";
const SHADOW = "name: demo-linear\ntitle: The project's linear\nstations: [{id: only, exit: [{kind: confirm}]}]\n";

let backend: Backend;
let api: TestClient;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startTestApp();
});
test.afterAll(async () => { await backend?.dispose(); });
test.beforeEach(async () => {
  api = await connect(backend);
});
test.afterEach(() => api?.close());

/** A project with its own pack, added as a project (untrusted, as every project starts). */
async function project(): Promise<{ root: string; id: string }> {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "harnix-own-flows-")));
  const dir = join(root, ".harnix", "flows");
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "proj-check.yaml"), OWN);
  writeFileSync(join(dir, "demo-linear.yaml"), SHADOW);
  writeFileSync(join(dir, "broken.yaml"), "name: [not a flow\n");
  const made = await api.call("harnix-app.workspaces.create", { path: root, title: "own flows" });
  return { root, id: made.workspace.id };
}

async function cleanUp(root: string, id: string, runs: string[]) {
  for (const run of runs) await api.call("flow.cancel", { run, reason: "test over" }).catch(() => undefined);
  await api.call("harnix-app.projects.trust", { project_root: root, trusted: false }).catch(() => undefined);
  await api.call("harnix-app.workspaces.remove", { id }).catch(() => undefined);
  rmSync(root, { recursive: true, force: true });
}

const row = (defs: any[], name: string) => defs.find((d) => d.name === name);

test("an untrusted project's own flows are listed unread, wait on trust, and are not started", TAGS, async () => {
  const { root, id } = await project();
  try {
    const status = await api.call("harnix-app.projects.status", { project_root: root });
    expect(status.trust).toBe("untrusted");

    const defs = await api.call("flow.defs", { project: root });
    // Listed by file name alone: no title, no digest — nothing of the project's is read.
    expect(row(defs, "proj-check")).toMatchObject({ origin: "project", valid: false, reason: "untrusted", title: "", digest: "" });
    // The project's name hides the built-in one in the project: "trust the project first".
    expect(row(defs, "demo-linear")).toMatchObject({ origin: "project", valid: false, reason: "untrusted" });
    expect(row(defs, "broken")).toMatchObject({ origin: "project", reason: "untrusted" });
    // The application's flows are there for the project as ever.
    expect(row(defs, "demo-review")).toMatchObject({ origin: "builtin", valid: true });

    for (const flow of ["proj-check", "demo-linear"]) {
      const e = await api.refused("flow.start", { flow, subject: "repo", project_root: root });
      expect(e?.code, flow).toBe(REFUSED);
      expect(e?.message).toMatch(/trust the project first/);
    }
    const e = await api.refused("flow.def", { name: "proj-check", project: root });
    expect(e?.code).toBe(REFUSED);

    // Outside the project nothing of it is there.
    const everywhere = await api.call("flow.defs");
    expect(row(everywhere, "proj-check")).toBeUndefined();
    expect(row(everywhere, "demo-linear")).toMatchObject({ origin: "builtin", valid: true });
  } finally {
    await cleanUp(root, id, []);
  }
});

test("a trusted project's own flow is offered to its tasks and starts one; the same name is the project's there", TAGS, async () => {
  const { root, id } = await project();
  const runs: string[] = [];
  try {
    const trusted = await api.call("harnix-app.projects.trust", { project_root: root, trusted: true });
    expect(trusted.trust).toBe("trusted");

    // Read again on the next listing: no restart.
    const defs = await api.call("flow.defs", { project: root });
    expect(row(defs, "proj-check")).toMatchObject({ origin: "project", valid: true, title: "The project's check" });
    expect(row(defs, "proj-check").reason).toBeUndefined();
    expect(row(defs, "demo-linear")).toMatchObject({ origin: "project", valid: true, title: "The project's linear" });
    // A file that is not a definition is reported, not offered.
    expect(row(defs, "broken")).toMatchObject({ origin: "project", valid: false, reason: "unreadable" });
    const info = await api.call("flow.def", { name: "proj-check", project: root });
    expect(info).toMatchObject({ origin: "project", valid: true });

    // A task of the project, from its own flow.
    const { run } = await api.call("flow.start", { flow: "proj-check", subject: "repo", project_root: root });
    runs.push(run);
    const { record } = await api.call("flow.get", { run });
    expect(record.def.name).toBe("proj-check");
    expect(record.project_root).toBe(root);
    expect((await api.call("flow.list", { project: root })).map((r: any) => r.id)).toContain(run);

    // The same name: the project's in the project, the built-in one elsewhere.
    const inProject = await api.call("flow.start", { flow: "demo-linear", subject: "repo", project_root: root });
    runs.push(inProject.run);
    expect((await api.call("flow.get", { run: inProject.run })).record.def.title).toBe("The project's linear");
    const builtIn = await api.call("flow.def", { name: "demo-linear" });
    expect(builtIn.origin).toBe("builtin");
    expect(builtIn.def.title).not.toBe("The project's linear");

    // No longer trusted: withheld again on the next listing.
    await api.call("harnix-app.projects.trust", { project_root: root, trusted: false });
    expect(row(await api.call("flow.defs", { project: root }), "proj-check")).toMatchObject({ valid: false, reason: "untrusted" });
  } finally {
    await cleanUp(root, id, runs);
  }
});
