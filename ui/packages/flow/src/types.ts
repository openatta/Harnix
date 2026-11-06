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

/** `{path, eq | ne | len | min_len | exists}`, exactly one of the comparisons. Shared by `when` and `tool` gates. */
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

/** What becomes of a tool call a permission matches (xops-04 §3). */
export type Effect = "allow" | "ask" | "deny";

/**
 * One permission: which tool calls (`tools`, a name or `ns__*`), in which scope (`paths`,
 * `commands`, `args`), and what becomes of them. Judged deny over ask over allow.
 */
export interface Permission {
  effect: Effect;
  tools: string[];
  paths?: string[] | null;
  commands?: string[] | null;
  args?: Record<string, string[]> | null;
  message?: string | null;
}

/** A permission with where it came from (`policy <name>`, `flow`, `station <id>`). */
export interface Granted extends Permission {
  from: string;
}

/**
 * How a session task picks its session: the run's main session (`main`, the default without a
 * `scene`), a new one (the default with a `scene`), one per run, or one per key.
 */
export type SessionAlloc = "main" | "new" | "per_run" | { key: string };

export type Executor =
  | {
      kind: "session";
      /** Absent: the task is a turn of the run's main session (xops-03 §1). */
      scene?: string | null;
      session?: SessionAlloc | null;
      /** Where the session runs and on what model (xops-02 §1); each optional. */
      model?: string | null;
      provider?: string | null;
      instance?: string | null;
      template?: string | null;
      params?: Record<string, string> | null;
      space?: string | null;
    }
  /** A sub-agent of the run's main session (xops-03 §2). */
  | { kind: "agent"; agent_type?: string | null; model?: string | null; description?: string | null }
  | { kind: "program"; step: string }
  | { kind: "external"; who?: string | null }
  | { kind: "subflow"; flow: string; inputs?: Record<string, unknown> | null };

export type ExecutorKind = Executor["kind"];

export type ExitCond =
  | { kind: "output" }
  | { kind: "gate"; gate: string; label?: string | null }
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
  /** Rules (`rules/<name>.md`) this station works under besides the flow's (xops-04 §2). */
  rules?: string[] | null;
  /** Policies this station works under besides the flow's. */
  policies?: string[] | null;
  /** Permissions written on the station itself. */
  permissions?: Permission[] | null;
}

/** Where a run's main session goes and on which model (xops-03 §1); each optional. */
export interface MainSession {
  scene?: string | null;
  model?: string | null;
  provider?: string | null;
  instance?: string | null;
  template?: string | null;
  params?: Record<string, string> | null;
  space?: string | null;
}

export interface FlowDef {
  name: string;
  title?: string | null;
  description?: string | null;
  inputs?: Record<string, InputDef> | null;
  main?: MainSession | null;
  rework_limit?: number | null;
  /** Rules, policies and permissions every station works under (xops-04). */
  rules?: string[] | null;
  policies?: string[] | null;
  permissions?: Permission[] | null;
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

/** Tokens a model spent, summed over turns (xops-03 §4). */
export interface Usage {
  input_tokens?: number | null;
  output_tokens?: number | null;
  cache_read_input_tokens?: number | null;
  cache_creation_input_tokens?: number | null;
  turns?: number | null;
}

/**
 * What a run is doing now (xops-03 §3): work in hand (`running`), waiting on a person or the
 * outside (`waiting`, `reason` says on what), paused with work still finishing (`pausing`), paused,
 * or over.
 */
export type PhaseState = "running" | "waiting" | "pausing" | "paused" | "finished" | "cancelled";

export interface Phase {
  state: PhaseState;
  reason?: "approval" | "confirm" | "held" | "unmet" | "failed" | "external" | "children" | "work" | string | null;
  /** The stations it is about: those working, or those waiting. */
  stations?: string[] | null;
}

/** A claim. Readers never get its token (the module removes it); a subflow task's runs until its child finishes. */
export interface Lease {
  by: Actor;
  token?: string | null;
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
  /** A dry run's session task: what its session would have been sent (xops-02 §5). */
  preview?: TaskPreview | null;
  /** The current attempt: claimed, and handed in or failed (xops-03 §4). */
  started_at?: Millis | null;
  ended_at?: Millis | null;
  usage?: Usage | null;
  /** A call of its turn held for a person's answer (an `ask` permission, xops-04 §3). */
  asking?: { tool: string; reason?: string | null; since: Millis } | null;
}

/** How a session task's output comes back: structured output (the default) or `flow_submit`. */
export type Delivery = "structured_output" | "flow_submit";

/**
 * A dry run's session task, mocked: the session it would have opened and the whole first message
 * it would have been sent (protocol layer and skill).
 */
export interface TaskPreview {
  round?: number | null;
  attempt?: number | null;
  scene?: string | null;
  message: string;
  placement?: Record<string, unknown> | null;
  delivery?: Delivery | string | null;
  output_schema?: JsonSchema | null;
  session_key?: string | null;
}

/** How an exit condition was judged: `wait` while a gate, a confirmation or children are outstanding. */
export type CheckStatus = "pass" | "fail" | "wait";

/**
 * One exit condition, judged (the kernel's `CheckRec`). `cond` is the condition's index in the
 * station's `exit`, absent for the station's tasks themselves; `kind` also takes `tasks` and `hold`
 * in a run view's `unmet`. `checkOk()` reads `status` (or a bare `ok`).
 */
export interface CheckRec {
  /** The condition's kind (`output`, `gate`, …, `tasks`, `hold`). */
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
  /** Session key → session; `@main` is the run's main session (xops-03 §1). */
  session_keys?: Record<string, string> | null;
  /** Set while the run is paused. */
  paused?: { by: Actor; at: Millis } | null;
  /** Turns of the main session that worked on no task. */
  usage_other?: Usage | null;
  /** The definition, pinned when the run started. */
  def?: FlowDef | null;
  /** The policies and gates the definition uses, pinned with it (xops-04). */
  parts?: { policies?: Record<string, PolicyDef> | null; gates?: Record<string, GateDef> | null } | null;
}

/**
 * `app.flow.get` and the `app.flow.run {run}` topic: the snapshot (its pinned definition in
 * `record.def`) and what each open station still lacks. `defOf()` reads the definition.
 */
export interface RunView {
  record: RunRecord;
  def?: FlowDef | null;
  unmet?: Record<string, Unmet[]> | null;
  /** What the run is doing (xops-03 §3), as its summary has it. */
  phase?: Phase | null;
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
  phase?: Phase | null;
  usage?: Usage | null;
  dry?: boolean | null;
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

/** Where a flow comes from: built in, a saved copy over a built-in one, or written here. */
export type Origin = "builtin" | "override" | "user";

/** One line of `app.flow.defs` and of the `app.flow.defs` topic. */
export interface DefSummary {
  name: string;
  title?: string | null;
  digest: string;
  origin?: Origin | null;
  /** False: it does not validate against this application's catalog (it cannot be started). */
  valid?: boolean | null;
}

/** The kinds of part a pack holds besides flows (xops-04 §1). */
export type PartKind = "skill" | "rule" | "policy" | "gate" | "hook";

/** A policy: a named set of permissions. */
export interface PolicyDef {
  name: string;
  title?: string | null;
  description?: string | null;
  permissions: Permission[];
}

/** A gate: a named check an exit `{kind: gate}` refers to. */
export interface GateDef {
  name: string;
  title?: string | null;
  description?: string | null;
  kind: "tool" | "command" | "host";
  tool?: string | null;
  args?: unknown;
  assert?: Assert[] | null;
  command?: string | null;
  cwd?: string | null;
  timeout_seconds?: number | null;
}

/** A hook over a flow, as `app.flow.def` lists it. */
export interface HookBrief {
  name: string;
  title?: string | null;
  on?: string[] | null;
  kind: string;
  action: string;
}

/** A hook's last delivery (`app.flow.hooks`). */
export interface HookDelivery {
  at: Millis;
  event: string;
  run?: string | null;
  status: "delivered" | "failed" | string;
  code?: number | null;
  error?: string | null;
  tries?: number | null;
}

/** One line of `app.flow.hooks`. */
export interface HookInfo extends HookBrief {
  description?: string | null;
  flows?: string[] | null;
  signed?: boolean | null;
  last?: HookDelivery | null;
}

/** One part, read (`app.flow.part {kind, name}`). */
export interface PartInfo {
  kind: PartKind;
  name: string;
  text: string;
  digest?: string | null;
  origin?: Origin | null;
  title?: string | null;
  description?: string | null;
  parsed?: unknown;
  problems?: string[] | null;
  flows?: string[] | null;
  writable?: boolean | null;
}

/** The texts of the parts a flow uses, by the kind's directory (`null`: not in the packs). */
export interface UsedParts {
  skills?: Record<string, string | null> | null;
  rules?: Record<string, string | null> | null;
  policies?: Record<string, string | null> | null;
  gates?: Record<string, string | null> | null;
}

/** `app.flow.def {name}`: the parsed definition, its YAML, its digest, the parts it uses. */
export interface DefInfo {
  def: FlowDef;
  yaml: string;
  digest: string;
  parts?: UsedParts | null;
  /** The hooks of the pack over this flow. */
  hooks?: HookBrief[] | null;
  /** Each station's rules (the flow's and its own) and its permissions in order (xops-04). */
  stations?: Record<string, { rules?: string[] | null; permissions?: Granted[] | null }> | null;
  /** The protocol layer each station's sessions get, by station id, when the module gives it. */
  protocol?: Record<string, string> | null;
  origin?: Origin | null;
  valid?: boolean | null;
  /** What keeps it from validating; empty when valid. */
  problems?: (Problem | string)[] | null;
  /** Whether the packs can be written (the conversation's tools save there). */
  writable?: boolean | null;
}

/** One line of `app.flow.dry_run.list` and of the `app.flow.dry_runs` topic. */
export interface DryRunSummary {
  id: string;
  flow: string;
  status: RunStatus;
  created_at: Millis;
  finished_at?: Millis | null;
  phase?: Phase | null;
}

/** One piece of work a dry run took: a mocked task, a gate… */
export interface DryStep {
  kind: string;
  run: string;
  station?: string | null;
  task?: string | null;
  gate?: string | null;
  result?: unknown;
}

/** `app.flow.dry_run.step` / `dry_run.run`: what was taken, what is still queued, the run's status. */
export interface DryProgress {
  run: string;
  steps: DryStep[];
  pending: number;
  status: RunStatus;
}

/** The `app.flow.focus {instance, session_id}` topic and the `focus` method's answer. */
export interface Focus {
  flow: string | null;
}

/** A session as the module names it on the wire. */
export interface SessionWire {
  instance: string;
  session_id: string;
}

/** `app.flow.session {flow?}`: a flow's own conversation, or (no flow) the general one. */
export interface FlowConversation extends SessionWire {
  flow?: string | null;
}

/** One of `session.tasks`: a main session's sub-agent, shell command, monitor… (Agix's shape). */
export interface SessionTask {
  task_id: string;
  kind: string;
  status: string;
  started_ms?: Millis | null;
  ended_ms?: Millis | null;
  detail?: { agent_type?: string | null; description?: string | null; command?: string | null; [k: string]: unknown } | null;
  error?: string | null;
  /** The flow task a sub-agent works on, read off its description. */
  flow?: { station: string; task: string } | null;
}

/** A sub-agent of a run's main session, as the session's events showed it (xops-03 §2). */
export interface RunAgent {
  agent_session_id: string;
  instance: string;
  agent_type?: string | null;
  tool_use_id?: string | null;
  description?: string | null;
  /** The flow task it works on, read off its description. */
  flow?: { station: string; task: string } | null;
  status?: "done" | "failed" | string | null;
}

/**
 * `app.flow.sessions {run}`: the run's main session, its sub-agents (`agents`), its sessions of
 * their own; and what Agix lists of the main session's tasks and side chains.
 */
export interface RunSessions {
  main?: SessionWire | null;
  agents?: RunAgent[] | null;
  own?: { station: string; task: string; session: SessionWire }[] | null;
  tasks?: SessionTask[] | null;
  sidechains?: ({ session_id: string; instance: string; name?: string | null; title?: string | null; created_at?: unknown } & Record<string, unknown>)[] | null;
}

/** `app.flow.guide {lang}`: the guide to flows, markdown. */
export interface Guide {
  text: string;
  lang: string;
}

/** An entry of the app's catalog: a bare name, or a name with what it is. */
export type CatalogEntry = string | { name: string; description?: string | null; [key: string]: unknown };

/** `app.flow.catalog`: what this application registered for definitions to name. */
export interface Catalog {
  scenes?: CatalogEntry[] | null;
  steps?: CatalogEntry[] | null;
  /** The application's own checks, which `host` gates name. */
  checks?: CatalogEntry[] | null;
  tools?: CatalogEntry[] | null;
  command_gates?: boolean | null;
  templates?: CatalogEntry[] | null;
  instances?: CatalogEntry[] | null;
  spaces?: CatalogEntry[] | null;
  models?: CatalogEntry[] | null;
  /** The tools refused unless a permission allows them. */
  guarded?: CatalogEntry[] | null;
  /** The parts on offer, by kind. */
  parts?: Partial<Record<PartKind, string[]>> | null;
  writable?: boolean | null;
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
  /** The session it was written in: where its acceptance is recorded. */
  session?: TaskSession | null;
}

/** An `app.flow` entry with `event: "proposal_handled"`: a proposal was accepted, once. */
export interface ProposalHandled {
  entry: string;
  action: "rework" | "cancel";
  by: Actor;
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
  | "cancel"
  | "proposal"
  | "focus"
  | "session"
  | "sessions"
  | "pause"
  | "resume"
  | "guide"
  | "parts"
  | "part"
  | "part.save"
  | "part.delete"
  | "hooks"
  | "dry_run.start"
  | "dry_run.step"
  | "dry_run.run"
  | "dry_run.list";
