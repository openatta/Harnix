/**
 * What a session sees beyond its conversation, and each extension's log, with the showcase
 * (`examples/packages/showcase`) installed: its skill in a session of its space is a skill of the
 * space tier, from its package's directory (`harnix.session.skills`); the hooks a session's turns
 * run are its event hook among this process's own, and a hook the project configures, said by its
 * kind and matcher only and held back while the project is untrusted (`harnix.session.hooks`);
 * its backend's own log is in its extension log (`system.extensions.log`), which a device reads
 * through the system space alone.
 *
 * On both carriers: the process carrier's package logs to its process's stderr, read as its lines.
 */

import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { REFUSED, SPACE_MISMATCH } from "@harnix/core/protocol";
import { type Backend, type TestClient, carrier, connect, expect, sleep, test, topology } from "@harnix/testkit";

import { SHOWCASE_ID, showcasePackage, startApps } from "./packages-host";

const TAGS = { tag: ["@feature:packages", "@layer:api"] };

const ON_PROCESS = carrier() === "process";
/** Where a session of its space runs, as a call names it. */
const AT = ON_PROCESS ? { instance: `proc.${SHOWCASE_ID}`, space: SHOWCASE_ID } : { instance: "main", space: SHOWCASE_ID };

/** A hook in the project's settings whose command, URL and header must never be shown. */
const SECRET = "hx-secret-4f1e";

test.describe("what a session sees, and each extension's log", () => {
  test.describe.configure({ mode: "serial" });

  let backend: Backend;
  let api: TestClient;

  test.beforeAll(async () => {
    test.setTimeout(600_000);
    showcasePackage();
    backend = await startApps();
    const client = await connect(backend);
    try {
      await client.call("packages.installFile", { path: showcasePackage().path });
    } finally {
      client.close();
    }
  });
  test.afterAll(async () => {
    await backend?.dispose();
  });
  test.beforeEach(async () => {
    api = await connect(backend, { topology: topology() });
  });
  test.afterEach(() => api?.close());

  /** A session of the showcase's space, in memory. */
  async function inItsSpace(): Promise<{ instance: string; session_id: string; space: string }> {
    const created = await api.call("harnix.session.create", { space: SHOWCASE_ID });
    const s = { ...AT, session_id: created.session_id };
    await api.call("harnix.session.attach", s);
    return s;
  }

  test("a session of its space sees its skill, from its package's directory", TAGS, async () => {
    const s = await inItsSpace();
    const seen = await api.call("harnix.session.skills", s);
    expect(seen).toMatchObject({ instance: AT.instance, session_id: s.session_id, listed: true });
    const tour = seen.skills.find((k: any) => k.name === "showcase-tour");
    expect(tour, JSON.stringify(seen.skills)).toMatchObject({ tier: "space", package: SHOWCASE_ID, withheld: false });
    expect(tour.dir).toMatch(new RegExp(`ext/${SHOWCASE_ID.replace(".", "\\.")}/skills$`));
    expect(seen.dirs.find((d: any) => d.package === SHOWCASE_ID)?.skills).toContain("showcase-tour");
  });

  test("a session of another space does not, and a claim that is not the session's is refused", TAGS, async () => {
    const created = await api.call("harnix.session.create", { instance: "main", project_root: api.project });
    const s = { instance: "main", session_id: created.session_id };
    await api.call("harnix.session.attach", s);
    const seen = await api.call("harnix.session.skills", s);
    expect(seen.skills.map((k: any) => k.name)).not.toContain("showcase-tour");
    expect((await api.refused("harnix.session.skills", { ...s, space: SHOWCASE_ID }))?.code).toBe(SPACE_MISMATCH);
    expect((await api.refused("harnix.session.hooks", { ...s, space: SHOWCASE_ID }))?.code).toBe(SPACE_MISMATCH);
  });

  test("the hooks a session runs: this process's own, and the project's held back while it is untrusted, never by what they run", TAGS, async () => {
    const settings = join(api.project, ".harnix");
    mkdirSync(settings, { recursive: true });
    writeFileSync(
      join(settings, "settings.json"),
      JSON.stringify({
        hooks_config: {
          PreToolUse: [{ type: "command", command: `echo ${SECRET}`, if: "Bash(git push:*)", on_error: "block" }],
          SessionEnd: [{ type: "http", url: `https://user:${SECRET}@hooks.example.com/x`, headers: { authorization: `Bearer ${SECRET}` } }],
        },
      }),
    );
    try {
      const created = await api.call("harnix.session.create", { instance: "main", project_root: api.project });
      const s = { instance: "main", session_id: created.session_id };
      await api.call("harnix.session.attach", s);
      const seen = await api.call("harnix.session.hooks", s);
      const text = JSON.stringify(seen);
      expect(text).not.toContain(SECRET);
      expect(text).not.toContain("hooks.example.com");
      // The showcase's event hook is answered here, before every tool call.
      const host = seen.hooks.find((h: any) => h.source === "host" && h.event === "PreToolUse");
      expect(host, text).toMatchObject({ kind: "host", matcher: "*", in_force: true });
      expect(host.providers.length).toBeGreaterThan(0);
      // Held back unless the engine says the project is trusted.
      const held = seen.trust !== "trusted";
      const project = seen.hooks.filter((h: any) => h.source === "project");
      expect(project.map((h: any) => [h.event, h.kind, h.label, h.withheld, h.in_force])).toEqual([
        ["PreToolUse", "command", "command · Bash(git push:*)", held, !held],
        ["SessionEnd", "http", "http", held, !held],
      ]);
    } finally {
      rmSync(join(settings, "settings.json"), { force: true });
    }
  });

  test("its backend's own log is its extension's, read through the system space", TAGS, async () => {
    // The backend says it started; waking it is enough.
    await api.call(`${SHOWCASE_ID}.state`);
    let lines: any[] = [];
    for (let i = 0; i < 40; i++) {
      const log = await api.call("system.extensions.log", { extension: SHOWCASE_ID });
      lines = log.lines;
      if (lines.some((l) => l.message.includes("the showcase's backend started"))) break;
      await sleep(250);
    }
    const started = lines.find((l) => l.message.includes("the showcase's backend started"));
    expect(started, JSON.stringify(lines.slice(-10))).toMatchObject({ extension: SHOWCASE_ID, level: "info", origin: ON_PROCESS ? "process" : "package" });
    // Every line asked for is its; asked for errors only, the info line is not there.
    expect(lines.every((l) => l.extension === SHOWCASE_ID)).toBe(true);
    const errors = await api.call("system.extensions.log", { extension: SHOWCASE_ID, level: "error" });
    expect(errors.lines.map((l: any) => l.message)).not.toContain(started.message);
    // From the newest line on, nothing older comes back.
    const after = await api.call("system.extensions.log", { after: started.seq });
    expect(after.lines.every((l: any) => l.seq > started.seq)).toBe(true);
    expect(after.extensions.map((c: any) => c.extension)).toContain(SHOWCASE_ID);
    // The kernel's row is for an extension granted to look into the process, not a device.
    expect((await api.refused("harnix.extensions.log", {}))?.code).toBe(REFUSED);
  });
});
