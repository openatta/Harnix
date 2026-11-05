/**
 * The flow module's wire shapes (xops-01 §3, §4, §6), as the interface reads them.
 *
 * The JSON is serde's output of `harnix-flow`'s Rust types (snake_case fields, enums tagged by
 * `kind`). Everything the interface assumes about it is in this one file, so a field the Rust side
 * names differently is changed here and nowhere else. Fields the interface does not need may be
 * absent; optional ones are typed `?` and may also arrive as `null`.
 */

/* ━━ ① The definition (YAML, parsed) ━━ */

/** A JSON Schema, as written in the definition. Not interpreted here beyond display. */
export type JsonSchema = Record<string, unknown>;

/** `{path, eq | ne | len | min_len | exists}`, exactly one of the comparisons. Shared by `when` and the `tool` exit. */
export interface Assert {
  path: string;
  eq?: unknown;
  ne?: unknown;
  len?: number;
  min_len?: number;
  exists?: boolean;
  items?: unknown;
}

export interface InputDef {
  schema?: JsonSchema | null;
  default?: unknown;
}

export interface HookRef {
  on: string;
  action: string;
}

/** How a session task picks its session: a new one (default), one per run, or one per key. */
export type SessionAlloc = "new" | "per_run" | { key: string };

export type Executor =
  | { kind: "session"; scene: string; template?: string | null; session?: SessionAlloc | null }
  | { kind: "program"; step: string }
  | { kind: "external"; who?: string | null }
  | { kind: "subflow"; flow: string; inputs?: Record<string, unknown> | null };

export type ExecutorKind = Executor["kind"];

export type ExitCond =
  | { kind: "output" }
  | { kind: "tool"; tool: string; args?: Record<string, unknown> | null; assert?: Assert[] | null }
  | { kind: "gate"; gate: string }
  | { kind: "confirm"; who?: string | null }
  | { kind: "children" };

export type ExitKind = ExitCond["kind"];

/** One of a station's static `tasks`. */
export interface TaskSpec {
  id: string;
  executor: Executor;
  skill?: string | null;
  retry?: { max: number } | null;
}

export interface OnUnmet {
  rework_to: string;
  max?: number | null;
}

/**
 * A station as written in YAML: its work is one of `executor` (one task), `tasks` (several, each with
 * an `id`), `foreach` + `executor` (one task per item), or none of them (a pure gate).
 */
export interface Station {
  id: string;
  title?: string | null;
  description?: string | null;
  /** Absent: the previous station in the list (none for the first). Present and empty: a root. */
  needs?: string[] | null;
  when?: Assert[] | null;
  executor?: Executor | null;
  tasks?: TaskSpec[] | null;
  /** A reference (`${stations.collect.output.modules}`) to the list the station fans out over. */
  foreach?: string | null;
  max_parallel?: number | null;
  skill?: string | null;
  output?: JsonSchema | null;
  exit?: ExitCond[] | null;
  retry?: { max: number } | null;
  rework_to?: string[] | null;
  on_unmet?: OnUnmet | null;
  hold?: boolean | null;
  writes?: string[] | null;
  hooks?: HookRef[] | null;
}

export interface FlowDef {
  name: string;
  title?: string | null;
  description?: string | null;
  inputs?: Record<string, InputDef> | null;
  rework_limit?: number | null;
  hooks?: HookRef[] | null;
  stations: Station[];
}

/* ━━ ② The run record (snapshot + history) ━━ */

export type RunStatus = "active" | "finished" | "cancelled";
export type StationStatus = "pending" | "held" | "working" | "unmet" | "passed" | "skipped" | "stale";
export type TaskStatus = "ready" | "claimed" | "submitted" | "failed" | "cancelled";

/** Who did something. The kernel does not interpret it: `{kind:"device", id}`, `{kind:"flow", id:"dispatch"}`… */
export interface Actor {
  kind: string;
  id: string;
}

/** Milliseconds since the epoch (the kernel's `Clock::now_ms`). An RFC 3339 string is read too. */
export type Millis = number;

export interface Lease {
  by: Actor;
  token: string;
  until: Millis;
}

/** The session a `session` task runs in. */
export interface TaskSession {
  instance: string;
  session_id: string;
}

/**
 * How a run record names a task's session. The kernel keeps it opaque; the flow module writes
 * `"<instance>/<session_id>"`. An object is read too. `sessionOf()` reads either.
 */
export type TaskSessionRef = string | TaskSession;

export interface TaskRec {
  id: string;
  item?: unknown;
  status: TaskStatus;
  attempt: number;
  lease?: Lease | null;
  output?: unknown;
  error?: string | null;
  session?: TaskSessionRef | null;
  /** The child run of a `subflow` task. */
  child?: string | null;
}

/** How an exit condition was judged: `wait` while a gate, a confirmation or children are outstanding. */
export type CheckStatus = "pass" | "fail" | "wait";

/**
 * One exit condition, judged (the kernel's `CheckRec`). `cond` is the condition's index in the
 * station's `exit`, absent for the station's tasks themselves; `kind` also takes `tasks` and `hold`
 * in a run view's `unmet`. `checkOk()` reads `status` (or a bare `ok`).
 */
export interface CheckRec {
  /** The condition's kind (`output`, `tool`, …, `tasks`, `hold`). */
  kind: ExitKind | string;
  cond?: number | ExitCond | null;
  label?: string | null;
  status?: CheckStatus;
  ok?: boolean;
  detail?: unknown;
  round?: number | null;
  at?: Millis | null;
}

export interface ConfirmRec {
  by: Actor;
  at: Millis;
  pass: boolean;
  reason?: string | null;
}

/** An exit condition that does not hold right now (`RunView.unmet`, `ForcedRec.unmet`): a check not passed. */
export interface Unmet {
  kind: ExitKind | string;
  cond?: number | ExitCond | null;
  label?: string | null;
  status?: CheckStatus;
  detail?: unknown;
}

export interface ForcedRec {
  by: Actor;
  at: Millis;
  reason: string;
  unmet?: Unmet[] | null;
}

export interface StationRec {
  status: StationStatus;
  round: number;
  rounds_total?: number | null;
  auto_reworks?: number | null;
  /** Skill name → the digest pinned when the station was entered (a bare digest is read too). */
  skill_digest?: Record<string, string> | string | null;
  /** Absent when the station has none (a pure checkpoint, or not entered yet). */
  tasks?: TaskRec[] | null;
  checks?: CheckRec[] | null;
  confirms?: ConfirmRec[] | null;
  forced?: ForcedRec | null;
  entered_at?: Millis | null;
  left_at?: Millis | null;
}

export interface ParentRef {
  run: string;
  station: string;
  task: string;
}

export interface RunRecord {
  id: string;
  flow: string;
  def_digest: string;
  title?: string | null;
  subject: string;
  inputs?: Record<string, unknown> | null;
  status: RunStatus;
  rev: number;
  created_at: Millis;
  created_by: Actor;
  finished_at?: Millis | null;
  parent?: ParentRef | null;
  stations: Record<string, StationRec>;
  session_keys?: Record<string, string> | null;
  /** The definition, pinned when the run started. */
  def?: FlowDef | null;
}

/**
 * `app.flow.get` and the `app.flow.run {run}` topic: the snapshot (its pinned definition in
 * `record.def`) and what each open station still lacks. `defOf()` reads the definition.
 */
export interface RunView {
  record: RunRecord;
  def?: FlowDef | null;
  unmet?: Record<string, Unmet[]> | null;
}

/** One line of `app.flow.list` and of the `app.flow.runs` topic. */
export interface RunSummary {
  id: string;
  flow: string;
  title?: string | null;
  subject: string;
  status: RunStatus;
  created_at: Millis;
  finished_at?: Millis | null;
  /** The stations entered and not finished (the kernel's `open`). */
  open?: string[] | null;
  /** The same, as an older shape named it. */
  current?: string[] | null;
}

export interface History {
  seq: number;
  at: Millis;
  by: Actor;
  op: string;
  station?: string | null;
  task?: string | null;
  detail?: unknown;
}

/* ━━ ③ The module's other answers ━━ */

/** One line of `app.flow.defs`. */
export interface DefSummary {
  name: string;
  title?: string | null;
  digest: string;
}

/** `app.flow.def {name}`: the parsed definition, its YAML, its digest, and the bodies of the skills it names. */
export interface DefInfo {
  def: FlowDef;
  yaml: string;
  digest: string;
  skills?: Record<string, string> | null;
  /** The protocol layer each station's sessions get, by station id, when the module gives it. */
  protocol?: Record<string, string> | null;
}

/** An entry of the app's catalog: a bare name, or a name with what it is. */
export type CatalogEntry = string | { name: string; description?: string | null; [key: string]: unknown };

/** `app.flow.catalog`: what this application registered for definitions to name. */
export interface Catalog {
  scenes?: CatalogEntry[] | null;
  steps?: CatalogEntry[] | null;
  gates?: CatalogEntry[] | null;
  tools?: CatalogEntry[] | null;
  actions?: CatalogEntry[] | null;
  skills?: CatalogEntry[] | null;
}

/** One structural problem from `app.flow.validate`. */
export interface Problem {
  /** A machine-readable code, when given. */
  code?: string | null;
  /** The station it is about, when it is about one. */
  station?: string | null;
  /** Where in the definition, as a path (`stations[2].exit[0]`), when given. */
  path?: string | null;
  message: string;
}

/** The payload of an `app.flow` entry with `event: "proposal"`, written by `flow_propose`. */
export interface Proposal {
  run: string;
  station: string;
  kind: "rework" | "cancel";
  to?: string | null;
  reason: string;
}

/** The app-side method names, relative to the module id (`app.<module>.<name>`). */
export type FlowMethod =
  | "list"
  | "get"
  | "history"
  | "start"
  | "defs"
  | "def"
  | "schema"
  | "catalog"
  | "validate"
  | "claim"
  | "heartbeat"
  | "submit"
  | "fail"
  | "confirm"
  | "force"
  | "rework"
  | "release"
  | "retry"
  | "cancel";
