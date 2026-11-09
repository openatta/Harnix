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
  Phase,
  PhaseState,
  RunRecord,
  Usage,
  FlowDef,
  FlowMethod,
  Granted,
  Permission,
  PartKind,
  PolicyDef,
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
  tool:
    | "create"
    | "update"
    | "delete"
    | "start"
    | "dry_run"
    | "part_set"
    | "part_delete"
    | "pause"
    | "resume"
    | "cancel"
    | "confirm"
    | "reject"
    | "force"
    | "rework"
    | "retry"
    | "release";
  /** The flow it acts on (create: the new flow's name). */
  flow?: string;
  /** The title the YAML it hands in gives (create, update). */
  title?: string;
  /** The part a `flow_part_*` call writes: its kind and name (xops-04). */
  part?: string;
  kind?: PartKind;
  subject?: string;
  /** A run tool's run, and the station (`to` for a rework) it acts on (xops-03 §5). */
  run?: string;
  station?: string;
}

const AUTHOR_TOOLS: Record<string, ToolTarget["tool"]> = {
  flow_create: "create",
  flow_update: "update",
  flow_delete: "delete",
  flow_start: "start",
  flow_dry_run: "dry_run",
  flow_part_set: "part_set",
  flow_part_delete: "part_delete",
  flow_pause: "pause",
  flow_resume: "resume",
  flow_cancel: "cancel",
  flow_confirm: "confirm",
  flow_force: "force",
  flow_rework: "rework",
  flow_retry: "retry",
  flow_release: "release",
};

/** The tools that act on a run rather than on a flow. */
export const RUN_TOOL_TARGETS: readonly ToolTarget["tool"][] = ["pause", "resume", "cancel", "confirm", "reject", "force", "rework", "retry", "release"];

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
 * name has to come from here. Null for any other tool. A name may arrive as
 * `<namespace>__flow_update`.
 */
export function toolTarget(name: string | undefined, input: unknown): ToolTarget | null {
  const bare = (name ?? "").split("__").pop() ?? "";
  let tool = AUTHOR_TOOLS[bare];
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
  if (tool === "confirm" && a.pass === false) tool = "reject";
  const out: ToolTarget = { tool };
  if (RUN_TOOL_TARGETS.includes(tool)) {
    out.run = text(a.run);
    out.station = text(tool === "rework" ? a.to : a.station);
    return out;
  }
  if (tool === "create") {
    out.flow = yamlField(a.yaml, "name");
    out.title = yamlField(a.yaml, "title");
  } else if (tool === "part_set" || tool === "part_delete") {
    out.part = text(a.name);
    const kind = text(a.kind);
    if (kind && (PART_KINDS as readonly string[]).includes(kind)) out.kind = kind as PartKind;
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

/* ── rules, policies, permissions, gates (xops-04) ───────────────────── */

export const PART_KINDS: readonly PartKind[] = ["skill", "rule", "policy", "gate", "hook"];

/** A part kind's directory in a pack, the key `app.flow.def`'s `parts` uses. */
export function partDir(kind: PartKind): "skills" | "rules" | "policies" | "gates" | "hooks" {
  return ({ skill: "skills", rule: "rules", policy: "policies", gate: "gates", hook: "hooks" } as const)[kind];
}

/** The rules a station works under: the flow's, then its own, each once (the kernel's order). */
export function rulesOf(def: FlowDef, s: Station): string[] {
  return [...new Set([...(def.rules ?? []), ...(s.rules ?? [])])];
}

/**
 * What a station's tasks work under, in the kernel's order: the flow's policies, the flow's own
 * permissions, the station's policies, its own. A policy not in `policies` contributes nothing.
 */
export function permissionsOf(def: FlowDef, s: Station, policies: Record<string, PolicyDef> | null | undefined): Granted[] {
  const out: Granted[] = [];
  const add = (names: string[] | null | undefined) => {
    for (const n of names ?? []) for (const p of policies?.[n]?.permissions ?? []) out.push({ ...p, from: `policy ${n}` });
  };
  add(def.policies);
  for (const p of def.permissions ?? []) out.push({ ...p, from: "flow" });
  add(s.policies);
  for (const p of s.permissions ?? []) out.push({ ...p, from: `station ${s.id}` });
  return out;
}

/** A permission's scope in one line: `paths a, b; commands c; table x`; empty for "every call". */
export function permissionScope(p: Permission): string {
  const parts: string[] = [];
  if (p.paths?.length) parts.push(`paths ${p.paths.join(", ")}`);
  if (p.commands?.length) parts.push(`commands ${p.commands.join(", ")}`);
  for (const [k, v] of Object.entries(p.args ?? {})) parts.push(`${k} ${v.join(", ")}`);
  return parts.join("; ");
}

/** The title of a part from its text: a YAML part's `title:`, a rule's first `# ` heading, a skill's `name:`. */
export function partTitle(kind: PartKind, name: string, text: string | null | undefined): string {
  if (!text) return name;
  if (kind === "rule") {
    const m = /^#\s+(.+?)\s*$/m.exec(text.trim().split("\n")[0] ?? "");
    return m ? m[1]! : name;
  }
  if (kind === "skill") return yamlField(text, "description") ?? name;
  return yamlField(text, "title") ?? name;
}

/** A gate's title: the run's pinned gate, else the pack's text the module gave; "" when neither says. */
export function gateTitle(gate: string, info: { parts?: { gates?: Record<string, string | null> | null } | null } | null | undefined, view?: { record: { parts?: { gates?: Record<string, { title?: string | null }> | null } | null } } | null): string {
  const pinned = view?.record.parts?.gates?.[gate]?.title;
  if (pinned) return pinned;
  const t = partTitle("gate", gate, info?.parts?.gates?.[gate]);
  return t === gate ? "" : t;
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

/* ── what a run is doing, and what it spent (xops-03 §3, §4) ─────────── */

/** The session key of a run's main session. */
export const MAIN_SESSION_KEY = "@main";

/** A run's main session, as `host.session.open` takes it; null before it is opened. */
export function mainSessionOf(record: Pick<RunRecord, "session_keys"> | null | undefined): { instance: string; sessionId: string } | null {
  const name = record?.session_keys?.[MAIN_SESSION_KEY];
  return name ? sessionOf({ session: name } as TaskRec) : null;
}

export const PHASE_STATES: readonly PhaseState[] = ["running", "waiting", "pausing", "paused", "finished", "cancelled"];

const PHASE_TONE: Record<PhaseState, Tone> = {
  running: "info",
  waiting: "warn",
  pausing: "warn",
  paused: "muted",
  finished: "ok",
  cancelled: "muted",
};

/** What a run (or a dry run's summary) is doing: its `phase`, or — from a shape without one — its status. */
export function phaseOf(r: { phase?: Phase | null; status?: RunStatus | string | null }): Phase {
  if (r.phase && PHASE_STATES.includes(r.phase.state)) return r.phase;
  if (r.status === "finished" || r.status === "cancelled") return { state: r.status };
  return { state: "running" };
}

export function phaseTone(state: PhaseState | string): Tone {
  return PHASE_TONE[state as PhaseState] ?? "muted";
}

/** All the tokens of a usage: input (cache included) and output. 0 for none. */
export function tokensOf(u: Usage | null | undefined): number {
  if (!u) return 0;
  return (u.input_tokens ?? 0) + (u.output_tokens ?? 0) + (u.cache_read_input_tokens ?? 0) + (u.cache_creation_input_tokens ?? 0);
}

/** Usages added up. */
export function addUsage(...us: (Usage | null | undefined)[]): Usage {
  const out = { input_tokens: 0, output_tokens: 0, cache_read_input_tokens: 0, cache_creation_input_tokens: 0, turns: 0 };
  for (const u of us) {
    if (!u) continue;
    out.input_tokens += u.input_tokens ?? 0;
    out.output_tokens += u.output_tokens ?? 0;
    out.cache_read_input_tokens += u.cache_read_input_tokens ?? 0;
    out.cache_creation_input_tokens += u.cache_creation_input_tokens ?? 0;
    out.turns += u.turns ?? 0;
  }
  return out;
}

/** A station's tokens: its tasks', summed. */
export function stationUsage(rec: StationRec | undefined): Usage {
  return addUsage(...(rec?.tasks ?? []).map((t) => t.usage));
}

/** A run's tokens: every task's and the main session's other turns. */
export function runUsage(record: RunRecord): Usage {
  return addUsage(record.usage_other, ...Object.values(record.stations ?? {}).map(stationUsage));
}

/** How long a station has been at it: entered → left, or → `now` while open; null if never entered. */
export function stationElapsed(rec: StationRec | undefined, now: number): number | null {
  const from = toMillis(rec?.entered_at);
  if (from === null) return null;
  const to = toMillis(rec?.left_at) ?? now;
  return Math.max(0, to - from);
}

/** How long a run has been at it: started → finished, or → `now`. */
export function runElapsed(r: { created_at?: unknown; finished_at?: unknown }, now: number): number | null {
  const from = toMillis(r.created_at);
  if (from === null) return null;
  return Math.max(0, (toMillis(r.finished_at) ?? now) - from);
}

function toMillis(at: unknown): number | null {
  if (typeof at === "number" && Number.isFinite(at)) return at;
  if (typeof at === "string" && at) {
    const n = Date.parse(at);
    return Number.isNaN(n) ? null : n;
  }
  return null;
}

/** `850ms`, `42s`, `3m 05s`, `2h 07m`; "" for none. */
export function formatDuration(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms)) return "";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, "0")}s`;
  return `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, "0")}m`;
}

/** `940`, `12.3k`, `1.2M`; "" for 0. */
export function formatTokens(n: number | null | undefined): string {
  if (!n) return "";
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

/* ── the guide (xops-03 §6) ──────────────────────────────────────────── */

/** One block of the guide's markdown, as the reference tab draws it. */
export type GuideBlock =
  | { kind: "heading"; level: number; text: string }
  | { kind: "paragraph"; text: string }
  | { kind: "list"; items: string[] }
  | { kind: "table"; head: string[]; rows: string[][] }
  /** `example`: a whole flow (YAML starting `name:`), which can be the start of a new one. */
  | { kind: "code"; lang: string; text: string; example: boolean };

/** The guide's markdown as blocks: headings, paragraphs, lists, tables and fenced code — all it uses. */
export function guideBlocks(markdown: string): GuideBlock[] {
  const lines = markdown.replace(/\r\n?/g, "\n").split("\n");
  const out: GuideBlock[] = [];
  let i = 0;
  // A row's cells: split at `|`, but not inside `code` nor at an escaped `\|`.
  const cells = (line: string) => {
    const row = line.trim().replace(/^\|/, "").replace(/\|$/, "");
    const out: string[] = [];
    let cell = "";
    let code = false;
    for (let k = 0; k < row.length; k++) {
      const c = row[k]!;
      if (c === "\\" && row[k + 1] === "|") {
        cell += "|";
        k++;
      } else if (c === "`") {
        code = !code;
        cell += c;
      } else if (c === "|" && !code) {
        out.push(cell.trim());
        cell = "";
      } else cell += c;
    }
    out.push(cell.trim());
    return out;
  };
  while (i < lines.length) {
    const line = lines[i]!;
    const fence = /^```(\w*)\s*$/.exec(line);
    if (fence) {
      const body: string[] = [];
      i++;
      while (i < lines.length && !/^```\s*$/.test(lines[i]!)) body.push(lines[i++]!);
      i++;
      const text = body.join("\n");
      out.push({ kind: "code", lang: fence[1] ?? "", text, example: fence[1] === "yaml" && /^name:/.test(text) });
      continue;
    }
    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      out.push({ kind: "heading", level: heading[1]!.length, text: heading[2]!.trim() });
      i++;
      continue;
    }
    if (/^\s*\|/.test(line)) {
      const rows: string[][] = [];
      while (i < lines.length && /^\s*\|/.test(lines[i]!)) rows.push(cells(lines[i++]!));
      const [head, sep, ...rest] = rows;
      const isSep = (r: string[] | undefined) => !!r && r.every((c) => /^:?-{2,}:?$/.test(c));
      out.push(isSep(sep) ? { kind: "table", head: head ?? [], rows: rest } : { kind: "table", head: [], rows });
      continue;
    }
    if (/^\s*(?:[-*]|\d+\.)\s+/.test(line)) {
      const items: string[] = [];
      while (i < lines.length && lines[i]!.trim() !== "" && !/^```/.test(lines[i]!) && !/^#/.test(lines[i]!)) {
        const l = lines[i++]!;
        const item = /^\s*(?:[-*]|\d+\.)\s+(.*)$/.exec(l);
        if (item) items.push(item[1]!.trim());
        else if (items.length) items[items.length - 1] += ` ${l.trim()}`;
      }
      out.push({ kind: "list", items });
      continue;
    }
    if (line.trim() === "") {
      i++;
      continue;
    }
    const para: string[] = [];
    while (i < lines.length && lines[i]!.trim() !== "" && !/^(```|#|\s*\||\s*(?:[-*]|\d+\.)\s)/.test(lines[i]!)) para.push(lines[i++]!.trim());
    out.push({ kind: "paragraph", text: para.join(" ") });
  }
  return out;
}

/** The guide's five groups (xops-04, the guide's `##` sections): 1 skeleton, 2 running, 3 constraints
 *  and checks, 4 interfaces to the outside, 5 writing and trying. */
export type GuideGroup = 1 | 2 | 3 | 4 | 5;

export const GUIDE_GROUPS: readonly GuideGroup[] = [1, 2, 3, 4, 5];

/** The group a `##` heading opens (`一、流程骨架`, `1. The skeleton`); null for any other. */
export function guideGroupOf(heading: string): GuideGroup | null {
  const t = heading.trim();
  const zh = /^([一二三四五])、/.exec(t);
  if (zh) return ("一二三四五".indexOf(zh[1]!) + 1) as GuideGroup;
  const en = /^([1-5])\.\s/.exec(t);
  return en ? (Number(en[1]) as GuideGroup) : null;
}

/** The term a guide table row is about: its first cell's first **bold** run, else the cell. */
export function guideTermOf(cell: string | undefined): string {
  const c = (cell ?? "").trim();
  const m = /\*\*(.+?)\*\*/.exec(c);
  return (m ? m[1]! : c).trim();
}

/** The guide's blocks with the group each is in: a `##` heading of a group opens it, any other
 *  `##` closes it. The overview list before the first group (five items) is the groups' own. */
export function guideSections(blocks: GuideBlock[]): { group: GuideGroup | null; blocks: GuideBlock[]; overview?: boolean }[] {
  const out: { group: GuideGroup | null; blocks: GuideBlock[]; overview?: boolean }[] = [{ group: null, blocks: [] }];
  for (const b of blocks) {
    if (b.kind === "heading" && b.level <= 2) {
      out.push({ group: b.level === 2 ? guideGroupOf(b.text) : null, blocks: [b] });
      continue;
    }
    out[out.length - 1]!.blocks.push(b);
  }
  for (const sec of out) {
    if (sec.group !== null) break;
    if (sec.blocks.some((b) => b.kind === "list" && b.items.length === 5)) sec.overview = true;
  }
  return out.filter((sec) => sec.blocks.length);
}

/** An inline run of guide text: plain, `code`, or **strong**. */
export interface Inline {
  kind: "text" | "code" | "strong";
  text: string;
}

export function inlineOf(text: string): Inline[] {
  const out: Inline[] = [];
  const re = /`([^`]+)`|\*\*([^*]+)\*\*/g;
  let at = 0;
  for (let m = re.exec(text); m; m = re.exec(text)) {
    if (m.index > at) out.push({ kind: "text", text: text.slice(at, m.index) });
    out.push(m[1] !== undefined ? { kind: "code", text: m[1] } : { kind: "strong", text: m[2]! });
    at = m.index + m[0].length;
  }
  if (at < text.length) out.push({ kind: "text", text: text.slice(at) });
  return out;
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
