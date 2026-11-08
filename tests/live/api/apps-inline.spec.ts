/**
 * Runtime applications, stage six (b): the inline points a manifest declares — `[[inline.tool_row]]`,
 * `[[inline.flow_block]]`, `[[inline.command]]`, `[[inline.sidebar_group]]` — which the shell draws
 * from the manifest alone, and `harnix.apps.clearData`.
 *
 * A manifest whose inline points do not hold together is refused by name, and nothing of it stays.
 * The showcase's, installed, are listed for the shell (`runtime.inline`) and disclosed where the
 * application is told something through them; its backend answers what the shell asks of it — the
 * candidates of its commands, its space's groups — and puts a card in a session's flow as an entry.
 * Clearing its data deletes what it keeps and nothing else, and starts it again from nothing.
 */

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { INVALID_PARAMS, REFUSED } from "@harnix/core/protocol";
import { type Backend, type TestClient, connect, expect, test, topology } from "@harnix/testkit";

import { SHOWCASE_ID, showcasePackage, startApps, variant } from "./apps-host";

const TAGS = { tag: ["@feature:apps-inline", "@layer:api"] };

const M = (name: string) => `app.${SHOWCASE_ID}.${name}`;

const listed = async (api: TestClient) => (await api.call("harnix.apps.list")).apps.find((a: any) => a.id === SHOWCASE_ID);

/** A session with one replayed turn: its log exists, so an entry has somewhere to go. */
async function oneTurn(api: TestClient, space?: string): Promise<{ instance: string; session_id: string }> {
  const created = await api.call("session.create", { ...(space ? { space } : {}), options: { recorder: { name: "says-ready" } } });
  const s = { instance: created.instance, session_id: created.session_id };
  await api.call("harnix.attach", s);
  await api.call("harnix.send", { ...s, message: "Say exactly: fixture ready" });
  await api.waitFor((f) => f.method === "harnix.turn_settled" && f.params.session_id === s.session_id, {
    timeout: 60_000,
    describe: "the turn to settle",
  });
  return s;
}

test.describe("an application's inline points and its data", () => {
  test.describe.configure({ mode: "serial" });

  let backend: Backend;
  let api: TestClient;
  let scratch: string;

  test.beforeAll(async () => {
    test.setTimeout(600_000);
    showcasePackage();
    scratch = mkdtempSync(join(tmpdir(), "hx-inline-"));
    backend = await startApps();
  });
  test.afterAll(async () => {
    await backend?.dispose();
    if (scratch) rmSync(scratch, { recursive: true, force: true });
  });
  test.beforeEach(async () => {
    api = await connect(backend, { topology: topology() });
  });
  test.afterEach(() => api?.close());

  test("inline points that do not hold together are refused by name, and nothing of it stays", TAGS, async () => {
    const edit = (from: string, to: string) => (e: Map<string, Buffer>) => {
      const manifest = e.get("harnix-app.toml")!.toString();
      expect(manifest, from).toContain(from);
      e.set("harnix-app.toml", Buffer.from(manifest.replace(from, to)));
    };
    const cases: [string, string, (e: Map<string, Buffer>) => void][] = [
      ["unknown-point", "unknown inline point `badge`", edit("[[inline.sidebar_group]]", "[[inline.badge]]")],
      ["unknown-field", "unknown field `colour`", edit('summary = "result"\n# The result', 'summary = "result"\ncolour = "red"\n# The result')],
      ["foreign-placeholder", "`{session.token}`", edit('title = "{payload.title}"', 'title = "{session.token}"')],
      ["undeclared-method", "`notes.purge`", edit('method = "counter.bump"', 'method = "notes.purge"')],
      ["no-complete", "`complete`", edit('name = "complete"', 'name = "completion"')],
      ["foreign-space", "the grouping of `other.space`", edit('space = "example.showcase"\nby', 'space = "other.space"\nby')],
      ["no-append", "harnix.entries.append", edit(', "harnix.entries.append"]', "]")],
      ["unknown-locale", "the locale `fr`", edit('zh-CN = "示例应用回声', 'fr = "示例应用回声')],
      // Only the component knows its tools: a row for one it does not list fails the start.
      ["unknown-tool", "does not list as a tool", edit('tool = "count"', 'tool = "counted"')],
    ];
    for (const [name, says, change] of cases) {
      const path = variant(join(scratch, `${name}.hxapp`), change);
      const error = await api.refused("harnix.apps.installFile", { path });
      expect(error?.code, name).toBe(REFUSED);
      expect(error?.message, name).toContain(says);
    }
    expect((await api.call("harnix.apps.list")).apps).toEqual([]);
  });

  test("installed, they are listed for the shell, and disclosed where they tell it something", TAGS, async () => {
    await api.call("harnix.apps.installFile", { path: showcasePackage().path });
    const app = await listed(api);
    const inline = app.runtime.inline;
    expect(inline.tool_row.map((r: any) => [r.tool, r.collapsed])).toEqual([
      ["echo", false],
      ["count", true],
    ]);
    expect(inline.tool_row[0].title).toMatchObject({ en: "Showcase echo · {input.text}" });
    expect(inline.flow_block[0]).toMatchObject({ kind: "card", title: "{payload.title}" });
    expect(inline.flow_block[0].button.map((b: any) => b.method)).toEqual(["counter.bump", "notes.add"]);
    expect(inline.command.map((c: any) => `${c.trigger}${c.name}`)).toEqual(["/showcase", "@notes"]);
    expect(inline.sidebar_group[0]).toMatchObject({ space: SHOWCASE_ID, by: "method", method: "sessions.group" });
    // What it declares is part of what it is, stopped or running.
    expect(app.app.inline).toEqual(inline);
    // It is told what is typed for its commands, and its space's sessions: each a line of its own.
    const lines = app.app.lines.filter((l: any) => l.capability === "inline").map((l: any) => l.value);
    expect(lines).toEqual(["command /showcase", "command @notes", `sidebar.group ${SHOWCASE_ID}`]);
    // Stopped, nothing is drawn for it.
    await api.call("harnix.apps.disable", { id: SHOWCASE_ID });
    expect((await listed(api)).runtime.inline).toBeUndefined();
    await api.call("harnix.apps.enable", { id: SHOWCASE_ID });
    expect((await listed(api)).runtime.inline).toEqual(inline);
  });

  test("its backend answers what the shell asks: command candidates, a card, its space's groups", TAGS, async () => {
    const slash = await api.call(M("complete"), { trigger: "/", name: "showcase", query: "", space: "sessions", session: null });
    expect(slash.candidates.map((c: any) => c.replacement)).toEqual(["/showcase-bump", "/showcase-clear"]);
    expect(slash.candidates[0]).toMatchObject({ call: { method: "counter.bump" } });
    await api.call(M("notes.add"), { text: "zebra crossing" });
    await api.call(M("notes.add"), { text: "another note" });
    const at = await api.call(M("complete"), { trigger: "@", name: "notes", query: "zeb" });
    expect(at.candidates.map((c: any) => c.label)).toEqual(["notes:zebra crossing"]);

    // A card: an entry under the application's own namespace, with the event its manifest draws.
    const carded = await oneTurn(api, SHOWCASE_ID);
    const plain = await oneTurn(api, SHOWCASE_ID);
    const posted = await api.call(M("card.post"), { ...carded, title: "inline-card-17" });
    expect(posted.appended).toMatchObject({ ns: `app.${SHOWCASE_ID}`, event: "card", payload: { title: "inline-card-17", notes: 2 } });
    const pushed = await api.waitFor(
      (f) => f.method === "harnix.event" && f.params.session_id === carded.session_id && f.params.event?.kind === "extension_entry",
      { describe: "the card's entry to be pushed" },
    );
    expect(pushed.params.event.entry.event).toBe("card");
    const { entries } = await api.call("session.entries", carded);
    expect(entries.map((e: any) => [e.ns, e.event])).toEqual([[`app.${SHOWCASE_ID}`, "card"]]);
    // Only through its `hub`: a session in which the entry is appended is still the session's.
    expect((await api.refused(M("card.post"), { title: "no session" }))?.code).toBe(INVALID_PARAMS);

    // Its space's groups: the sessions it put a card in, the rest left to the shell's `other`.
    const { sessions } = await api.call("harnix.sessions", { space: SHOWCASE_ID });
    const ids = sessions.map((r: any) => r.session_id);
    expect(ids).toEqual(expect.arrayContaining([carded.session_id, plain.session_id]));
    const grouped = await api.call(M("sessions.group"), { space: SHOWCASE_ID, sessions: sessions.map((r: any) => ({ session_id: r.session_id })) });
    expect(grouped.groups).toEqual([{ key: "cards", title: { en: "With a card", "zh-CN": "有卡片的" }, sessions: [carded.session_id] }]);
  });

  test("clearData deletes what it keeps and nothing else, and starts it again from nothing", TAGS, async () => {
    await api.call(M("counter.bump"));
    expect((await api.call(M("notes.list"))).notes.length).toBeGreaterThan(0);
    const before = (await listed(api)).data_bytes;
    expect(before).toBeGreaterThan(0);
    const cleared = await api.call("harnix.apps.clearData", { id: SHOWCASE_ID });
    expect(cleared).toMatchObject({ id: SHOWCASE_ID, cleared_bytes: before, data_bytes: 0, restarted: true });
    const after = await listed(api);
    expect(after).toMatchObject({ enabled: true, version: "0.1.0", data_bytes: 0, runtime: { state: "running" } });
    expect((await api.call(M("notes.list"))).notes).toEqual([]);
    // Started again: its memory went too.
    expect((await api.call(M("counter.bump"))).counter).toBe(1);
    // On the record, as something that cannot be undone.
    const audited = (await api.call("harnix.audit", { limit: 1000 })).entries.filter((e: any) => e.method === "harnix.apps.clearData");
    expect(audited.length).toBeGreaterThanOrEqual(1);
    // Disabled, it is cleared and stays off.
    await api.call(M("notes.add"), { text: "kept while off" });
    await api.call("harnix.apps.disable", { id: SHOWCASE_ID });
    expect(await api.call("harnix.apps.clearData", { id: SHOWCASE_ID })).toMatchObject({ data_bytes: 0, restarted: false });
    expect((await listed(api)).enabled).toBe(false);
    await api.call("harnix.apps.enable", { id: SHOWCASE_ID });
    expect((await api.call(M("notes.list"))).notes).toEqual([]);

    expect((await api.refused("harnix.apps.clearData", { id: "nobody.here" }))?.code).toBe(INVALID_PARAMS);
    expect((await api.refused("harnix.apps.clearData", {}))?.message).toContain("missing id");
  });
});
