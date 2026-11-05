/**
 * The pure helpers: a station's work, a status's look, which actions a station allows, the
 * proposal entry, the schema as a field list, and the two dictionaries.
 */

import { describe, expect, it } from "vitest";

import {
  methodName,
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

  it("confirm and force while working or unmet; confirm only with a confirm exit", () => {
    const r = table(review);
    for (const s of STATION_STATUSES) {
      const open = s === "working" || s === "unmet";
      expect(r[s]!.confirm, s).toBe(open);
      expect(r[s]!.force, s).toBe(open);
    }
    expect(stationActions("active", analyze, rec("working")).confirm).toBe(false);
  });

  it("rework to the station's targets while open, held or passed", () => {
    const r = table(review);
    expect(r.working!.rework).toEqual(["analyze", "lint"]);
    expect(r.passed!.rework).toEqual(["analyze", "lint"]);
    expect(r.held!.rework).toEqual(["analyze", "lint"]);
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
  });

  it("claims only the flow's proposal entries", () => {
    const entry = (ns: string, event: string, payload: unknown) => ({ kind: "custom", type: "entry", data: { id: "e1", ns, event, payload, at: 0 } });
    const p = { run: "r1", station: "review", kind: "rework", to: "analyze", reason: "findings are thin" };
    expect(proposalOf(entry("app.flow", "proposal", p))).toEqual({ id: "e1", proposal: p });
    expect(proposalOf(entry("app.flow", "proposal", { ...p, kind: "cancel", to: undefined }))?.proposal.kind).toBe("cancel");
    expect(proposalOf(entry("app.other", "proposal", p))).toBe(null);
    expect(proposalOf(entry("app.ops", "proposal", p), "app.ops")).not.toBe(null);
    expect(proposalOf(entry("app.flow", "other", p))).toBe(null);
    expect(proposalOf(entry("app.flow", "proposal", { ...p, kind: "delete" }))).toBe(null);
    expect(proposalOf({ kind: "custom", type: "request", data: {} })).toBe(null);
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
