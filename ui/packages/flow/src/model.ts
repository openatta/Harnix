/**
 * Pure helpers over the wire shapes: what a station's work is, how a status is drawn, which
 * actions a station allows, a JSON Schema as a field list. No React, no DOM.
 */

import type {
  CatalogEntry,
  CheckRec,
  DefSummary,
  DryRunSummary,
  Problem,
  ExecutorKind,
  FlowDef,
  FlowMethod,
  Proposal,
  ProposalHandled,
  RunStatus,
  RunSummary,
  RunView,
  Station,
  StationRec,
  StationStatus,
  TaskRec,
  TaskStatus,
} from "./types";

/* ── names on the wire ───────────────────────────────────────────────── */

/** `app.<module>.<name>`: the module id is the one the application registered it under. */
export function methodName(module: string, name: FlowMethod): string {
  return `app.${module}.${name}`;
}

export function runsTopic(module: string): string {
  return `app.${module}.runs`;
}

/** Subscribed with `{run}`. */
export function runTopic(module: string): string {
  return `app.${module}.run`;
}

/** The flows' list, re-sent after every save and delete. */
export function defsTopic(module: string): string {
  return `app.${module}.defs`;
}

/** Subscribed with `{instance, session_id}`: the flow that conversation works on. */
export function focusTopic(module: string): string {
  return `app.${module}.focus`;
}

/** The dry runs kept. */
export function dryRunsTopic(module: string): string {
  return `app.${module}.dry_runs`;
}

/** A dry run's id: the sandbox's runs are `dry-…`. */
export function isDryRun(run: string | null | undefined): boolean {
  return typeof run === "string" && run.startsWith("dry-");
}

/** The defs method's or topic's value: the list itself, or `{defs: […]}`. */
export function defsOf(value: unknown): DefSummary[] {
  if (Array.isArray(value)) return value as DefSummary[];
  const defs = (value as { defs?: unknown } | null)?.defs;
  return Array.isArray(defs) ? (defs as DefSummary[]) : [];
}

/** The dry runs' list: the list itself, or `{runs: […]}`. */
export function dryRunsOf(value: unknown): DryRunSummary[] {
  if (Array.isArray(value)) return value as DryRunSummary[];
  const runs = (value as { runs?: unknown } | null)?.runs;
  return Array.isArray(runs) ? (runs as DryRunSummary[]) : [];
}

/** What one of `flow-author`'s changing tools acts on, read off its call's input. */
export interface ToolTarget {
  tool: "create" | "update" | "delete" | "start" | "dry_run" | "skill_set" | "skill_delete";
  /** The flow it acts on (create: the new flow's name). */
  flow?: string;
  /** The title the YAML it hands in gives (create, update). */
  title?: string;
  skill?: string;
  subject?: string;
}

const AUTHOR_TOOLS: Record<string, ToolTarget["tool"]> = {
  flow_create: "create",
  flow_update: "update",
  flow_delete: "delete",
  flow_start: "start",
  flow_dry_run: "dry_run",
  flow_skill_set: "skill_set",
  flow_skill_delete: "skill_delete",
};

/** A top-level `key: value` of a YAML text, unquoted; undefined when it has none. */
function yamlField(yaml: unknown, key: string): string | undefined {
  if (typeof yaml !== "string") return undefined;
  const m = new RegExp(`^${key}:[ \t]*(.+?)[ \t]*$`, "m").exec(yaml);
  if (!m) return undefined;
  const v = m[1]!.replace(/^(["'])(.*)\1$/, "$2").trim();
  return v || undefined;
}

/**
 * The flow (or skill) a `flow-author` tool call acts on, from the call's input — what the
 * permission to run it is about. The engine asks "Allow flow_update?" whatever the tool says, so the
 * name has to come from here (xops-02 review #4). Null for any other tool. A name may arrive as
 * `<namespace>__flow_update`.
 */
export function toolTarget(name: string | undefined, input: unknown): ToolTarget | null {
  const bare = (name ?? "").split("__").pop() ?? "";
  const tool = AUTHOR_TOOLS[bare];
  if (!tool) return null;
  let args = input;
  if (typeof args === "string") {
    try {
      args = JSON.parse(args);
    } catch {
      args = {};
    }
  }
  const a = (args && typeof args === "object" ? args : {}) as Record<string, unknown>;
  const text = (v: unknown) => (typeof v === "string" && v.trim() ? v.trim() : undefined);
  const out: ToolTarget = { tool };
  if (tool === "create") {
    out.flow = yamlField(a.yaml, "name");
    out.title = yamlField(a.yaml, "title");
  } else if (tool === "skill_set" || tool === "skill_delete") {
    out.skill = text(a.name);
  } else {
    out.flow = text(a.flow);
    if (tool === "update") out.title = yamlField(a.yaml, "title");
    if (tool === "start") out.subject = text(a.subject);
  }
  return out;
}

/** A problem as one line: `station path: message`. */
export function problemText(p: Problem | string): string {
  if (typeof p === "string") return p;
  return [p.station, p.path].filter(Boolean).join(" ") + (p.station || p.path ? ": " : "") + p.message;
}

/** The run id in what `start` answered: an id, `{run}`, `{id}`, `{value: …}` or a run view. */
export function runIdOf(answer: unknown): string | null {
  if (typeof answer === "string") return answer;
  if (!answer || typeof answer !== "object") return null;
  const a = answer as Record<string, unknown>;
  if (typeof a.run === "string") return a.run;
  if (typeof a.id === "string") return a.id;
  if (a.record && typeof a.record === "object") return runIdOf(a.record);
  if (a.value !== undefined) return runIdOf(a.value);
  return null;
}

/** The runs topic's value: the list itself, or `{runs: […]}`. */
export function runsOf(value: unknown): RunSummary[] {
  if (Array.isArray(value)) return value as RunSummary[];
  const runs = (value as { runs?: unknown } | null)?.runs;
  return Array.isArray(runs) ? (runs as RunSummary[]) : [];
}

/** A run view's pinned definition: `record.def`, or a `def` beside the record. */
export function defOf(view: Pick<RunView, "record" | "def"> | null | undefined): FlowDef | undefined {
  return view?.def ?? view?.record?.def ?? undefined;
}

/** The stations a run summary says are open. */
export function openOf(r: RunSummary): string[] {
  return r.open ?? r.current ?? [];
}

/** Whether a judged condition passed: `status: "pass"`, or a bare `ok`. */
export function checkOk(c: Pick<CheckRec, "status" | "ok">): boolean {
  return c.status !== undefined ? c.status === "pass" : !!c.ok;
}

/** A station's pinned skill digests as `name: digest` lines (a bare digest as itself). */
export function digestLines(d: Record<string, string> | string | null | undefined): string[] {
  if (!d) return [];
  if (typeof d === "string") return [d];
  return Object.entries(d).map(([name, digest]) => `${name}: ${digest}`);
}

/* ── a station's work ────────────────────────────────────────────────── */

export interface WorkSummary {
  /** `none`: a pure gate. */
  shape: "none" | "one" | "tasks" | "foreach";
  /** The executor kinds, without repeats, in order. */
  kinds: ExecutorKind[];
  /** How many tasks the definition fixes; null for `foreach`, whose count is known only in a run. */
  count: number | null;
  skills: string[];
}

export function workOf(s: Station): WorkSummary {
  const uniq = <T,>(xs: T[]) => [...new Set(xs)];
  if (s.tasks && s.tasks.length) {
    return {
      shape: "tasks",
      kinds: uniq(s.tasks.map((t) => t.executor.kind)),
      count: s.tasks.length,
      skills: uniq(s.tasks.map((t) => t.skill).filter((x): x is string => !!x)),
    };
  }
  if (s.executor) {
    return { shape: s.foreach ? "foreach" : "one", kinds: [s.executor.kind], count: s.foreach ? null : 1, skills: s.skill ? [s.skill] : [] };
  }
  return { shape: "none", kinds: [], count: 0, skills: [] };
}

/** One short line: `session`, `program ×N`, `session+external ×2`, or "" for a pure gate. */
export function workLabel(s: Station): string {
  const w = workOf(s);
  if (w.shape === "none") return "";
  const kinds = w.kinds.join("+");
  if (w.shape === "foreach") return `${kinds} ×N`;
  if (w.shape === "tasks") return `${kinds} ×${w.count}`;
  return kinds;
}

export function exitKinds(s: Station): string[] {
  return [...new Set((s.exit ?? []).map((c) => c.kind))];
}

export function stationTitle(s: Pick<Station, "id" | "title">): string {
  return s.title?.trim() || s.id;
}

/** Where a station may send work back to, by hand: its `rework_to` and its `on_unmet` target. */
export function reworkTargets(s: Station): string[] {
  return [...new Set([...(s.rework_to ?? []), ...(s.on_unmet?.rework_to ? [s.on_unmet.rework_to] : [])])];
}

/* ── how a status is drawn ───────────────────────────────────────────── */

export type Tone = "muted" | "info" | "warn" | "error" | "ok";

export const STATION_STATUSES: readonly StationStatus[] = ["pending", "held", "working", "unmet", "passed", "skipped", "stale"];

const STATION_TONE: Record<StationStatus, Tone> = {
  pending: "muted",
  held: "warn",
  working: "info",
  unmet: "error",
  passed: "ok",
  skipped: "muted",
  stale: "muted",
};

const TASK_TONE: Record<TaskStatus, Tone> = {
  ready: "muted",
  claimed: "info",
  submitted: "ok",
  failed: "error",
  cancelled: "muted",
};

const RUN_TONE: Record<RunStatus, Tone> = { active: "info", finished: "ok", cancelled: "muted" };

/** A station status's tone and the classes that draw it (`hx-flow-tone-*` colours, `hx-flow-st-*` the rest). */
export function stationStyle(status: StationStatus | undefined): { tone: Tone; className: string } {
  if (!status || !(status in STATION_TONE)) return { tone: "muted", className: "hx-flow-tone-muted" };
  const tone = STATION_TONE[status];
  return { tone, className: `hx-flow-tone-${tone} hx-flow-st-${status}` };
}

export function taskTone(status: TaskStatus | string): Tone {
  return TASK_TONE[status as TaskStatus] ?? "muted";
}

export function runTone(status: RunStatus | string): Tone {
  return RUN_TONE[status as RunStatus] ?? "muted";
}

/** Submitted tasks of all, this round. */
export function taskProgress(rec: StationRec | undefined): { done: number; total: number } {
  const tasks = rec?.tasks ?? [];
  return { done: tasks.filter((t) => t.status === "submitted").length, total: tasks.length };
}

/* ── which actions a station allows ──────────────────────────────────── */

export interface StationActions {
  /** Confirm and reject: the station has a `confirm` exit and is being worked on or waiting. */
  confirm: boolean;
  /** Force it through: being worked on or waiting, conditions not all met. */
  force: boolean;
  /** Where it may send work back (empty: no rework from here). */
  rework: string[];
  /** Let a held station in. */
  release: boolean;
  /** Failed tasks that may be tried again. */
  retry: string[];
}

const NONE: StationActions = { confirm: false, force: false, rework: [], release: false, retry: [] };

export function stationActions(runStatus: RunStatus | undefined, def: Station | undefined, rec: StationRec | undefined): StationActions {
  if (runStatus !== "active" || !def || !rec) return NONE;
  const open = rec.status === "working" || rec.status === "unmet";
  // As the kernel has it: an open station (held included) may be forced, and may send work
  // back to what its `rework_to` / `on_unmet` names; a passed one no longer can.
  const entered = open || rec.status === "held";
  const confirmable = (def.exit ?? []).some((c) => c.kind === "confirm");
  return {
    confirm: open && confirmable,
    force: entered,
    rework: entered ? reworkTargets(def) : [],
    release: rec.status === "held",
    retry: open ? (rec.tasks ?? []).filter((t) => t.status === "failed").map((t) => t.id) : [],
  };
}

/** A task's session, as `host.session.open` takes it; null when it has none. */
export function sessionOf(task: TaskRec): { instance: string; sessionId: string } | null {
  const s = task.session as unknown;
  if (typeof s === "string") {
    // `"<instance>/<session_id>"`, as the flow module writes it; a bare id is on `system`.
    const at = s.indexOf("/");
    if (!s) return null;
    return at > 0 && at < s.length - 1 ? { instance: s.slice(0, at), sessionId: s.slice(at + 1) } : { instance: "system", sessionId: s };
  }
  if (!s || typeof s !== "object") return null;
  const o = s as Record<string, unknown>;
  const instance = o.instance;
  const id = o.session_id ?? o.sessionId ?? o.id;
  return typeof instance === "string" && typeof id === "string" ? { instance, sessionId: id } : null;
}

/** A lease's end, or null when it has none: a subflow task holds its claim until the child finishes. */
export function leaseUntil(until: unknown): number | null {
  return typeof until === "number" && until < 8.64e15 ? until : null;
}

/* ── the proposal card ───────────────────────────────────────────────── */

/** The `custom` block an entry becomes, as far as this package needs it. */
export interface BlockShape {
  kind: string;
  type?: string;
  data?: unknown;
}

/** The flow's proposal in a conversation block, if that is what it is. */
export function proposalOf(block: BlockShape, ns = "app.flow"): { id: string; proposal: Proposal } | null {
  if (block.kind !== "custom" || block.type !== "entry") return null;
  const entry = block.data as { id?: string; ns?: string; event?: string; payload?: unknown } | null;
  if (!entry || entry.ns !== ns || entry.event !== "proposal") return null;
  const p = entry.payload as Partial<Proposal> | null;
  if (!p || typeof p.run !== "string" || (p.kind !== "rework" && p.kind !== "cancel")) return null;
  const session = p.session && typeof p.session.instance === "string" && typeof p.session.session_id === "string" ? p.session : null;
  return {
    id: String(entry.id ?? ""),
    proposal: { run: p.run, station: String(p.station ?? ""), kind: p.kind, to: p.to ?? null, reason: String(p.reason ?? ""), session },
  };
}

/** A `proposal_handled` entry in a conversation block, if that is what it is. */
export function proposalHandledOf(block: BlockShape, ns = "app.flow"): ProposalHandled | null {
  if (block.kind !== "custom" || block.type !== "entry") return null;
  const entry = block.data as { ns?: string; event?: string; payload?: unknown } | null;
  if (!entry || entry.ns !== ns || entry.event !== "proposal_handled") return null;
  const p = entry.payload as Partial<ProposalHandled> | null;
  if (!p || typeof p.entry !== "string" || (p.action !== "rework" && p.action !== "cancel")) return null;
  return { entry: p.entry, action: p.action, by: p.by ?? { kind: "", id: "" } };
}

/* ── formatting ──────────────────────────────────────────────────────── */

/** A time from the record: ms since the epoch, or an RFC 3339 string. "" for none. */
export function formatTime(at: unknown): string {
  if (at === null || at === undefined || at === "") return "";
  const d = new Date(typeof at === "number" || typeof at === "string" ? at : NaN);
  if (Number.isNaN(d.getTime())) return String(at);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

export function pretty(value: unknown): string {
  if (value === undefined) return "";
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

export function actorLabel(a: { kind?: string; id?: string } | null | undefined): string {
  if (!a) return "";
  return a.kind ? `${a.kind}:${a.id ?? ""}` : String(a.id ?? "");
}

export function catalogEntries(list: CatalogEntry[] | null | undefined): { name: string; description: string }[] {
  return (list ?? []).map((e) => (typeof e === "string" ? { name: e, description: "" } : { name: String(e.name), description: e.description ?? "" }));
}

/* ── a JSON Schema as a list of fields ───────────────────────────────── */

export interface SchemaRow {
  /** `stations[].exit[]`, `stations[].executor`… A variant's own row carries `· <tag>`. */
  path: string;
  type: string;
  description: string;
  required: boolean;
  depth: number;
}

type S = Record<string, unknown>;

/**
 * The fields a JSON Schema describes, depth first, as a reader looks them up: each property with its
 * type and description; arrays as `name[]`; a map's values as `name.<key>`; a tagged union's
 * variants each on their own row, named by their tag (`kind = session`). References are followed
 * (`#/$defs/…`, `#/definitions/…`), once per branch, so a recursive schema ends.
 */
export function schemaRows(schema: unknown, maxDepth = 8): SchemaRow[] {
  const root = (schema && typeof schema === "object" ? schema : {}) as S;
  const defs = { ...((root.definitions as S | undefined) ?? {}), ...((root.$defs as S | undefined) ?? {}) };
  const rows: SchemaRow[] = [];

  const refName = (ref: unknown) => (typeof ref === "string" ? ref.replace(/^#\/(\$defs|definitions)\//, "") : "");

  /** Follow `$ref`, a lone `allOf`, and `Option`'s `anyOf [x, null]`; keep the outermost description. */
  const resolve = (node: S, seen: Set<string>): { node: S; name: string; description: string; seen: Set<string> } => {
    let n = node;
    let name = "";
    let description = typeof node.description === "string" ? node.description : "";
    let s = seen;
    for (let i = 0; i < 16; i += 1) {
      const ref = refName(n.$ref);
      if (ref && !s.has(ref) && defs[ref]) {
        s = new Set(s).add(ref);
        name = ref;
        n = defs[ref] as S;
      } else if (Array.isArray(n.allOf) && n.allOf.length === 1) {
        n = n.allOf[0] as S;
      } else {
        const alts = (n.anyOf ?? n.oneOf) as S[] | undefined;
        const nonNull = Array.isArray(alts) ? alts.filter((a) => a.type !== "null") : [];
        if (Array.isArray(alts) && alts.length === 2 && nonNull.length === 1 && !n.properties) n = nonNull[0]!;
        else break;
      }
      if (!description && typeof n.description === "string") description = n.description;
    }
    return { node: n, name, description, seen: s };
  };

  const tagOf = (variant: S): string => {
    const props = (variant.properties ?? {}) as Record<string, S>;
    for (const [k, p] of Object.entries(props)) {
      if (p.const !== undefined) return `${k} = ${String(p.const)}`;
      if (Array.isArray(p.enum) && p.enum.length === 1) return `${k} = ${String(p.enum[0])}`;
    }
    if (variant.const !== undefined) return JSON.stringify(variant.const);
    if (Array.isArray(variant.enum)) return variant.enum.map((x) => JSON.stringify(x)).join(" | ");
    if (Array.isArray(variant.required) && variant.required.length === 1) return String(variant.required[0]);
    return typeof variant.type === "string" ? variant.type : "…";
  };

  const typeOf = (n: S, name: string): string => {
    if (n.const !== undefined) return JSON.stringify(n.const);
    if (Array.isArray(n.enum)) return n.enum.map((x) => JSON.stringify(x)).join(" | ");
    const t = Array.isArray(n.type) ? (n.type as string[]).filter((x) => x !== "null").join(" | ") : (n.type as string | undefined);
    if (t === "array") {
      const items = n.items && typeof n.items === "object" ? resolve(n.items as S, new Set()) : null;
      return `array<${items ? items.name || typeOf(items.node, "") : "any"}>`;
    }
    if (t) return name && t === "object" ? name : t;
    if (n.anyOf || n.oneOf) return name || "one of";
    return name || "any";
  };

  const walk = (raw: S, path: string, depth: number, required: boolean, seen: Set<string>) => {
    const { node, name, description, seen: s } = resolve(raw, seen);
    if (path) rows.push({ path, type: typeOf(node, name), description, required, depth });
    if (depth >= maxDepth) return;
    const props = node.properties as Record<string, S> | undefined;
    if (props) {
      const req = new Set(Array.isArray(node.required) ? (node.required as string[]) : []);
      for (const [k, child] of Object.entries(props)) walk(child, path ? `${path}.${k}` : k, depth + 1, req.has(k), s);
    }
    const extra = node.additionalProperties;
    if (extra && typeof extra === "object" && !props) walk(extra as S, `${path}.<key>`, depth + 1, false, s);
    const items = node.items;
    if (items && typeof items === "object" && !Array.isArray(items)) {
      const inner = resolve(items as S, s);
      if (inner.node.properties || inner.node.oneOf || inner.node.anyOf) {
        const before = rows.length;
        walk(items as S, `${path}[]`, depth, false, s);
        // The items' own row repeats the array's: keep only their fields.
        if (rows[before]?.path === `${path}[]`) rows.splice(before, 1);
      }
    }
    const variants = (node.oneOf ?? node.anyOf) as S[] | undefined;
    if (Array.isArray(variants) && !(variants.length === 2 && variants.some((v) => v.type === "null"))) {
      for (const v of variants) {
        const r = resolve(v, s);
        if (r.node.type === "null") continue;
        const tag = tagOf(r.node);
        rows.push({ path: `${path} · ${tag}`, type: "variant", description: r.description, required: false, depth: depth + 1 });
        const vprops = r.node.properties as Record<string, S> | undefined;
        if (vprops && depth + 1 < maxDepth) {
          const req = new Set(Array.isArray(r.node.required) ? (r.node.required as string[]) : []);
          for (const [k, child] of Object.entries(vprops)) {
            if (child.const !== undefined || (Array.isArray(child.enum) && child.enum.length === 1)) continue; // the tag itself
            walk(child, path ? `${path}.${k}` : k, depth + 2, req.has(k), r.seen);
          }
        }
      }
    }
  };

  walk(root, "", 0, true, new Set());
  return rows;
}
