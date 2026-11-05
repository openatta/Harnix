/**
 * The pure helpers: a station's work, a status's look, which actions a station allows, the
 * proposal entry, the schema as a field list, and the two dictionaries.
 */

import { describe, expect, it } from "vitest";

import {
  formatDuration,
  formatTokens,
  guideBlocks,
  inlineOf,
  mainSessionOf,
  phaseOf,
  phaseTone,
  runElapsed,
  runUsage,
  stationElapsed,
  stationUsage,
  tokensOf,
  toolTarget,
  checkOk,
  leaseUntil,
  proposalHandledOf,
  defOf,
  digestLines,
  methodName,
  openOf,
  proposalOf,
  runIdOf,
  runsOf,
  schemaRows,
  sessionOf,
  STATION_STATUSES,
  stationActions,
  stationStyle,
  taskProgress,
  workLabel,
  workOf,
} from "../src/model";
import { STRINGS, STRINGS_OWNER } from "../src/strings";
import type { Station, StationRec, StationStatus, TaskRec } from "../src/types";

const task = (id: string, status: TaskRec["status"], extra: Partial<TaskRec> = {}): TaskRec => ({ id, status, attempt: 1, ...extra });
const rec = (status: StationStatus, tasks: TaskRec[] = []): StationRec => ({ status, round: 1, tasks });

const review: Station = { id: "review", needs: ["self-check"], exit: [{ kind: "confirm", who: "reviewer" }], rework_to: ["analyze", "lint"] };
const analyze: Station = {
  id: "analyze",
  foreach: "${stations.collect.output.modules}",
  executor: { kind: "session", scene: "flow-chat" },
  skill: "analyze",
  exit: [{ kind: "output" }],
  on_unmet: { rework_to: "collect" },
};

describe("work", () => {
  it("tells one executor, static tasks, foreach and a pure gate apart", () => {
    expect(workOf({ id: "a", executor: { kind: "program", step: "collect" } })).toEqual({ shape: "one", kinds: ["program"], count: 1, skills: [] });
    expect(workOf(analyze)).toEqual({ shape: "foreach", kinds: ["session"], count: null, skills: ["analyze"] });
    const report: Station = {
      id: "report",
      tasks: [
        { id: "summary", executor: { kind: "session", scene: "flow-chat" }, skill: "report" },
        { id: "sign", executor: { kind: "external", who: "reviewer" } },
      ],
    };
    expect(workOf(report)).toEqual({ shape: "tasks", kinds: ["session", "external"], count: 2, skills: ["report"] });
    expect(workOf(review).shape).toBe("none");
    expect(workLabel(analyze)).toBe("session ×N");
    expect(workLabel(report)).toBe("session+external ×2");
    expect(workLabel(review)).toBe("");
  });
});

describe("status style", () => {
  it("gives every station status a tone and its own class", () => {
    const tones = Object.fromEntries(STATION_STATUSES.map((s) => [s, stationStyle(s).tone]));
    expect(tones).toEqual({ pending: "muted", held: "warn", working: "info", unmet: "error", passed: "ok", skipped: "muted", stale: "muted" });
    for (const s of STATION_STATUSES) expect(stationStyle(s).className).toBe(`hx-flow-tone-${tones[s]} hx-flow-st-${s}`);
    expect(stationStyle(undefined).className).toBe("hx-flow-tone-muted");
    expect(stationStyle("bogus" as StationStatus).tone).toBe("muted");
  });

  it("counts submitted tasks", () => {
    expect(taskProgress(rec("working", [task("a", "submitted"), task("b", "claimed"), task("c", "submitted")]))).toEqual({ done: 2, total: 3 });
    expect(taskProgress(undefined)).toEqual({ done: 0, total: 0 });
  });
});

describe("actions per status", () => {
  const table = (def: Station, tasks: TaskRec[] = []) =>
    Object.fromEntries(STATION_STATUSES.map((s) => [s, stationActions("active", def, rec(s, tasks))]));

  it("confirm while working or unmet, force also while held (as the kernel allows); confirm only with a confirm exit", () => {
    const r = table(review);
    for (const s of STATION_STATUSES) {
      const open = s === "working" || s === "unmet";
      expect(r[s]!.confirm, s).toBe(open);
      expect(r[s]!.force, s).toBe(open || s === "held");
    }
    expect(stationActions("active", analyze, rec("working")).confirm).toBe(false);
  });

  it("rework to the station's targets while it is open or held — the kernel needs an open station listing the target", () => {
    const r = table(review);
    expect(r.working!.rework).toEqual(["analyze", "lint"]);
    expect(r.unmet!.rework).toEqual(["analyze", "lint"]);
    expect(r.held!.rework).toEqual(["analyze", "lint"]);
    expect(r.passed!.rework).toEqual([]);
    expect(r.pending!.rework).toEqual([]);
    expect(r.stale!.rework).toEqual([]);
    expect(r.skipped!.rework).toEqual([]);
    // on_unmet's target counts as one
    expect(stationActions("active", analyze, rec("unmet")).rework).toEqual(["collect"]);
  });

  it("release only a held station; retry only failed tasks of an open one", () => {
    const tasks = [task("m1", "failed"), task("m2", "submitted"), task("m3", "failed")];
    const r = table(analyze, tasks);
    for (const s of STATION_STATUSES) expect(r[s]!.release, s).toBe(s === "held");
    expect(r.working!.retry).toEqual(["m1", "m3"]);
    expect(r.unmet!.retry).toEqual(["m1", "m3"]);
    expect(r.passed!.retry).toEqual([]);
  });

  it("nothing on a run that is not active, or a station not entered", () => {
    for (const run of ["finished", "cancelled", undefined] as const) {
      expect(stationActions(run, review, rec("working"))).toEqual({ confirm: false, force: false, rework: [], release: false, retry: [] });
    }
    expect(stationActions("active", review, undefined).force).toBe(false);
  });
});

describe("wire helpers", () => {
  it("names methods under the module's id", () => {
    expect(methodName("flow", "confirm")).toBe("app.flow.confirm");
    expect(methodName("ops", "start")).toBe("app.ops.start");
  });

  it("finds the run id in what start answers", () => {
    expect(runIdOf("r-1")).toBe("r-1");
    expect(runIdOf({ run: "r-2" })).toBe("r-2");
    expect(runIdOf({ value: { run: "r-3" } })).toBe("r-3");
    expect(runIdOf({ record: { id: "r-4" } })).toBe("r-4");
    expect(runIdOf(null)).toBe(null);
  });

  it("reads the runs topic as a list or {runs}", () => {
    const one = { id: "r", flow: "f", subject: "s", status: "active", created_at: 1 };
    expect(runsOf([one])).toEqual([one]);
    expect(runsOf({ runs: [one] })).toEqual([one]);
    expect(runsOf(undefined)).toEqual([]);
  });

  it("reads a task's session", () => {
    expect(sessionOf(task("a", "claimed", { session: { instance: "i", session_id: "s" } }))).toEqual({ instance: "i", sessionId: "s" });
    expect(sessionOf(task("a", "claimed"))).toBe(null);
    // As the flow module writes it.
    expect(sessionOf(task("a", "claimed", { session: "system/abc" }))).toEqual({ instance: "system", sessionId: "abc" });
    expect(sessionOf(task("a", "claimed", { session: "abc" }))).toEqual({ instance: "system", sessionId: "abc" });
  });

  it("reads the kernel's shapes: the pinned definition, open stations, check status, digests", () => {
    const def = { name: "f", stations: [] };
    const record = { id: "r", flow: "f", def_digest: "d", subject: "s", status: "active" as const, rev: 1, created_at: 1, created_by: { kind: "device", id: "x" }, stations: {} };
    expect(defOf({ record: { ...record, def } })).toBe(def);
    expect(defOf({ record, def })).toBe(def);
    expect(openOf({ id: "r", flow: "f", subject: "s", status: "active", created_at: 1, open: ["a"] })).toEqual(["a"]);
    expect(openOf({ id: "r", flow: "f", subject: "s", status: "active", created_at: 1 })).toEqual([]);
    expect([checkOk({ status: "pass" }), checkOk({ status: "wait" }), checkOk({ status: "fail" }), checkOk({ ok: true })]).toEqual([true, false, false, true]);
    expect(digestLines({ analyze: "abc" })).toEqual(["analyze: abc"]);
    expect(digestLines(null)).toEqual([]);
    // A station without tasks has none on the wire.
    expect(stationActions("active", review, { status: "working", round: 1 }).confirm).toBe(true);
  });

  it("claims only the flow's proposal entries", () => {
    const entry = (ns: string, event: string, payload: unknown) => ({ kind: "custom", type: "entry", data: { id: "e1", ns, event, payload, at: 0 } });
    const p = { run: "r1", station: "review", kind: "rework", to: "analyze", reason: "findings are thin" };
    expect(proposalOf(entry("app.flow", "proposal", p))).toEqual({ id: "e1", proposal: { ...p, session: null } });
    const session = { instance: "system", session_id: "s1" };
    expect(proposalOf(entry("app.flow", "proposal", { ...p, session }))?.proposal.session).toEqual(session);
    expect(proposalOf(entry("app.flow", "proposal", { ...p, kind: "cancel", to: undefined }))?.proposal.kind).toBe("cancel");
    expect(proposalOf(entry("app.other", "proposal", p))).toBe(null);
    expect(proposalOf(entry("app.ops", "proposal", p), "app.ops")).not.toBe(null);
    expect(proposalOf(entry("app.flow", "other", p))).toBe(null);
    expect(proposalOf(entry("app.flow", "proposal", { ...p, kind: "delete" }))).toBe(null);
    expect(proposalOf({ kind: "custom", type: "request", data: {} })).toBe(null);
  });

  it("reads an accepted proposal's record, and nothing else", () => {
    const entry = (event: string, payload: unknown) => ({ kind: "custom", type: "entry", data: { id: "e2", ns: "app.flow", event, payload, at: 0 } });
    const by = { kind: "device", id: "d" };
    expect(proposalHandledOf(entry("proposal_handled", { entry: "e1", action: "rework", by }))).toEqual({ entry: "e1", action: "rework", by });
    expect(proposalHandledOf(entry("proposal", { entry: "e1", action: "rework", by }))).toBe(null);
    expect(proposalHandledOf(entry("proposal_handled", { action: "rework" }))).toBe(null);
  });

  it("draws a subflow task's lease as lasting until its child finishes", () => {
    expect(leaseUntil(1_800_000_000_000)).toBe(1_800_000_000_000);
    expect(leaseUntil(18446744073709551615)).toBe(null);
    expect(leaseUntil(undefined)).toBe(null);
  });
});

describe("schema as fields", () => {
  // The shape schemars gives: $defs, $ref, Option as anyOf [x, null], an internally tagged enum as oneOf.
  const schema = {
    title: "FlowDef",
    type: "object",
    required: ["name", "stations"],
    properties: {
      name: { type: "string", description: "The flow's name." },
      title: { type: ["string", "null"] },
      stations: { type: "array", items: { $ref: "#/$defs/Station" } },
    },
    $defs: {
      Station: {
        type: "object",
        required: ["id"],
        properties: {
          id: { type: "string", description: "Unique in the flow." },
          executor: { anyOf: [{ $ref: "#/$defs/Executor" }, { type: "null" }], description: "Who does the work." },
          next: { $ref: "#/$defs/Station" },
        },
      },
      Executor: {
        oneOf: [
          { type: "object", description: "A headless session.", required: ["kind", "scene"], properties: { kind: { const: "session" }, scene: { type: "string" } } },
          { type: "object", required: ["kind", "step"], properties: { kind: { type: "string", enum: ["program"] }, step: { type: "string" } } },
        ],
      },
    },
  };

  it("lists properties with types, descriptions, required, and follows refs and variants", () => {
    const rows = schemaRows(schema);
    const by = Object.fromEntries(rows.map((r) => [r.path, r]));
    expect(by.name).toMatchObject({ type: "string", description: "The flow's name.", required: true, depth: 1 });
    expect(by.title).toMatchObject({ type: "string", required: false });
    expect(by.stations).toMatchObject({ type: "array<Station>", required: true });
    expect(by["stations[].id"]).toMatchObject({ type: "string", description: "Unique in the flow.", required: true });
    expect(by["stations[].executor"]).toMatchObject({ type: "Executor", description: "Who does the work." });
    expect(by["stations[].executor · kind = session"]).toMatchObject({ type: "variant", description: "A headless session." });
    expect(by["stations[].executor · kind = program"]).toBeDefined();
    expect(by["stations[].executor.scene"]).toMatchObject({ type: "string", required: true });
    expect(by["stations[].executor.step"]).toBeDefined();
    // the tag is the variant's row, not a field of its own
    expect(by["stations[].executor.kind"]).toBeUndefined();
  });

  it("ends on a recursive schema", () => {
    const rows = schemaRows(schema);
    expect(rows.filter((r) => r.path.startsWith("stations[].next")).length).toBeLessThan(10);
  });

  it("reads `definitions` as well as `$defs`, and nothing from nothing", () => {
    const rows = schemaRows({ type: "object", properties: { a: { $ref: "#/definitions/A" } }, definitions: { A: { type: "integer", description: "an A" } } });
    expect(rows).toEqual([{ path: "a", type: "integer", description: "an A", required: false, depth: 1 }]);
    expect(schemaRows(null)).toEqual([]);
  });
});

describe("dictionaries", () => {
  const placeholders = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
  const en = STRINGS.en;
  const zh = STRINGS["zh-CN"];

  it("are under the package's owner, with the same keys and placeholders, none blank", () => {
    expect(STRINGS_OWNER).toBe("flowui");
    expect(Object.keys(en).filter((k) => !k.startsWith("flowui."))).toEqual([]);
    expect(Object.keys(zh).sort()).toEqual(Object.keys(en).sort());
    expect(Object.keys(en).filter((k) => JSON.stringify(placeholders(en[k]!)) !== JSON.stringify(placeholders(zh[k] ?? "")))).toEqual([]);
    expect([...Object.values(en), ...Object.values(zh)].filter((v) => !v.trim())).toEqual([]);
  });

  it("cover every status, exit kind and action the components look up", () => {
    const keys = [
      ...STATION_STATUSES.map((s) => `flowui.status.${s}`),
      ...["ready", "claimed", "submitted", "failed", "cancelled"].map((s) => `flowui.task.${s}`),
      ...["active", "finished", "cancelled"].map((s) => `flowui.runStatus.${s}`),
      ...["output", "tool", "gate", "confirm", "children"].map((k) => `flowui.exit.${k}`),
      ...["reject", "force", "rework", "cancel"].map((k) => `flowui.act.submit.${k}`),
      ...["scenes", "steps", "gates", "tools", "actions", "skills"].map((k) => `flowui.ref.cat.${k}`),
      "flowui.ref.of.definition",
      "flowui.ref.of.run",
    ];
    expect(keys.filter((k) => !(k in en))).toEqual([]);
  });
});

describe("what a run is doing and what it spent (xops-03)", () => {
  it("reads the phase, or falls back on the status", () => {
    expect(phaseOf({ phase: { state: "pausing" }, status: "active" }).state).toBe("pausing");
    expect(phaseOf({ status: "finished" }).state).toBe("finished");
    expect(phaseOf({ status: "active" }).state).toBe("running");
    expect(phaseOf({ phase: { state: "bogus" as never }, status: "cancelled" }).state).toBe("cancelled");
    expect(phaseTone("waiting")).toBe("warn");
  });

  it("adds tokens up by task, station and run, and formats them", () => {
    const rec = {
      usage_other: { input_tokens: 5, output_tokens: 5, turns: 1 },
      stations: {
        a: { status: "passed", round: 1, tasks: [{ id: "a", status: "submitted", attempt: 1, usage: { input_tokens: 100, output_tokens: 20, cache_read_input_tokens: 80, turns: 2 } }] },
        b: { status: "pending", round: 0 },
      },
    } as never;
    expect(tokensOf(stationUsage((rec as { stations: Record<string, never> }).stations.a))).toBe(200);
    expect(tokensOf(runUsage(rec))).toBe(210);
    expect(runUsage(rec).turns).toBe(3);
    expect(formatTokens(0)).toBe("");
    expect(formatTokens(940)).toBe("940");
    expect(formatTokens(12_345)).toBe("12k");
    expect(formatTokens(1_500)).toBe("1.5k");
    expect(formatTokens(2_300_000)).toBe("2.3M");
  });

  it("times a station and a run, open ones up to now", () => {
    expect(stationElapsed({ status: "passed", round: 1, entered_at: 1000, left_at: 4500 }, 9999)).toBe(3500);
    expect(stationElapsed({ status: "working", round: 1, entered_at: 1000 }, 3000)).toBe(2000);
    expect(stationElapsed({ status: "pending", round: 0 }, 3000)).toBe(null);
    expect(runElapsed({ created_at: 0, finished_at: 61_000 }, 0)).toBe(61_000);
    expect(formatDuration(850)).toBe("850ms");
    expect(formatDuration(42_000)).toBe("42s");
    expect(formatDuration(185_000)).toBe("3m 05s");
    expect(formatDuration(7_620_000)).toBe("2h 07m");
    expect(formatDuration(null)).toBe("");
  });

  it("finds the main session under @main", () => {
    expect(mainSessionOf({ session_keys: { "@main": "box/m1", run: "system/x" } })).toEqual({ instance: "box", sessionId: "m1" });
    expect(mainSessionOf({ session_keys: {} })).toBe(null);
  });

  it("names the run a run tool acts on", () => {
    expect(toolTarget("flow_pause", { run: "r-1" })).toEqual({ tool: "pause", run: "r-1", station: undefined });
    expect(toolTarget("flow_confirm", { run: "r-1", station: "sign", pass: false })).toMatchObject({ tool: "reject", run: "r-1", station: "sign" });
    expect(toolTarget("flow_confirm", { run: "r-1", station: "sign", pass: true })).toMatchObject({ tool: "confirm" });
    expect(toolTarget("flow_rework", { run: "r-1", to: "draft" })).toMatchObject({ tool: "rework", station: "draft" });
    expect(toolTarget("flow_runs", {})).toBe(null);
  });
});

describe("the guide's markdown", () => {
  it("is headings, paragraphs, lists, tables and code; a whole flow is an example", () => {
    const blocks = guideBlocks("# Title\n\nSome **bold** and `code`\ncontinued.\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n1. first\n2. second\n   more\n\n```yaml\nname: x\nstations: []\n```\n\n```yaml\n  - id: part\n```\n");
    expect(blocks.map((b) => b.kind)).toEqual(["heading", "paragraph", "table", "list", "code", "code"]);
    expect(blocks[1]).toEqual({ kind: "paragraph", text: "Some **bold** and `code` continued." });
    expect(blocks[2]).toEqual({ kind: "table", head: ["a", "b"], rows: [["1", "2"]] });
    expect(blocks[3]).toEqual({ kind: "list", items: ["first", "second more"] });
    expect(blocks[4]).toMatchObject({ example: true, text: "name: x\nstations: []" });
    expect(blocks[5]).toMatchObject({ example: false });
    expect(inlineOf("a `b` **c**")).toEqual([
      { kind: "text", text: "a " },
      { kind: "code", text: "b" },
      { kind: "text", text: " " },
      { kind: "strong", text: "c" },
    ]);
  });
});
