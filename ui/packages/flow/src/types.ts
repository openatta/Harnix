/**
 * The flow module's wire shapes, as the interface reads them.
 *
 * The definitions, the run records and the module's own rows and params are generated from the
 * Rust types that serialize them (`./generated/flow.ts`, by `crates/harnix/tests/ts_export.rs`) and
 * re-exported here under the names the interface uses. What the module builds by hand —
 * `flow.def`, `flow.part`, `flow.hooks`, `flow.sessions`, `flow.catalog`, the proposal entries —
 * and Agix's shapes are declared here, loosely: fields the interface does not need may be absent.
 */

import type { Actor, CheckRec, Executor, ExitCond, FlowDef, Granted, Origin, PartKind, Phase, Problem, SessionBinding, TaskRec as TaskRecord } from "./generated/flow";

export type {
  // ① The definition (YAML, parsed).
  Assert,
  InputDef,
  Effect,
  Permission,
  Granted,
  SessionBinding,
  Executor,
  ExitCond,
  TaskSpec,
  Retry,
  OnUnmet,
  Station,
  MainSession,
  FlowDef,
  Guard,
  // The parts of a pack.
  PartKind,
  PolicyDef,
  GateKind,
  GateDef,
  HookKind,
  HookDef,
  Pinned,
  // ② The run record (snapshot + history).
  RunStatus,
  StationStatus,
  TaskStatus,
  Actor,
  Usage,
  Phase,
  Lease,
  AskRec,
  CheckStatus,
  CheckRec,
  ConfirmRec,
  ForcedRec,
  StationRec,
  ParentRef,
  PauseRec,
  Workspace,
  Trigger,
  RunRecord,
  RunView,
  RunSummary,
  RunQuery,
  History,
  // ③ The module's rows and params.
  Origin,
  Delivery,
  Problem,
  DefRow,
  StartParams,
  StartTrigger,
  DryRunSummary,
  DryStep,
  DryProgress,
} from "./generated/flow";

/** A JSON Schema, as written in the definition. Not interpreted here beyond display. */
export type JsonSchema = Record<string, unknown>;

/**
 * How a session task picks its session: the run's main session (`main`, the default without a
 * `scene`), a new one (the default with a `scene`), one per run, or one per key.
 */
export type SessionAlloc = SessionBinding;

export type ExecutorKind = Executor["kind"];
export type ExitKind = ExitCond["kind"];

/** Milliseconds since the epoch (the kernel's `Clock::now_ms`). */
export type Millis = number;

/** What a run is doing now (xops-03 §3). */
export type PhaseState = Phase["state"];

/** The session a `session` task runs in. */
export interface TaskSession {
  instance: string;
  session_id: string;
}

/**
 * How a run record names a task's session: the flow module writes `"<instance>/<session_id>"`.
 * `sessionOf()` reads it.
 */
export type TaskSessionRef = string;

/** A task as a run view carries it: a dry run's session task also has its `preview`. */
export type TaskRec = TaskRecord & {
  /** A dry run's session task: what its session would have been sent (xops-02 §5). */
  preview?: TaskPreview | null;
};

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
  delivery?: string | null;
  output_schema?: JsonSchema | null;
  session_key?: string | null;
}

/** An exit condition that does not hold right now (`RunView.unmet`, `ForcedRec.unmet`): a check not passed. */
export type Unmet = CheckRec;

/** One line of `flow.defs` and of the `flow.defs` topic. */
export type { DefRow as DefSummary } from "./generated/flow";

/* ━━ ③ The module's other answers ━━ */

/** A hook over a flow, as `flow.def` lists it. */
export interface HookBrief {
  name: string;
  title?: string | null;
  on?: string[] | null;
  kind: string;
  action: string;
}

/** A hook's last delivery (`flow.hooks`). */
export interface HookDelivery {
  at: Millis;
  event: string;
  run?: string | null;
  status: "delivered" | "failed" | string;
  code?: number | null;
  error?: string | null;
  tries?: number | null;
}

/** One line of `flow.hooks`. */
export interface HookInfo extends HookBrief {
  description?: string | null;
  flows?: string[] | null;
  signed?: boolean | null;
  last?: HookDelivery | null;
}

/** One part, read (`flow.part {kind, name}`). */
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

/** `flow.def {name}`: the parsed definition, its YAML, its digest, the parts it uses. */
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

/** The `flow.focus {instance, session_id}` topic and the `focus` method's answer. */
export interface Focus {
  flow: string | null;
}

/** A session as the module names it on the wire. */
export interface SessionWire {
  instance: string;
  session_id: string;
}

/** `flow.session {flow?}`: a flow's own conversation, or (no flow) the general one. */
export interface FlowConversation extends SessionWire {
  flow?: string | null;
}

/** One of `engine.session.tasks`: a main session's sub-agent, shell command, monitor… (Agix's shape). */
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
 * `flow.sessions {run}`: the run's main session, its sub-agents (`agents`), its sessions of
 * their own; and what Agix lists of the main session's tasks and side chains.
 */
export interface RunSessions {
  main?: SessionWire | null;
  agents?: RunAgent[] | null;
  own?: { station: string; task: string; session: SessionWire }[] | null;
  tasks?: SessionTask[] | null;
  sidechains?: ({ session_id: string; instance: string; name?: string | null; title?: string | null; created_at?: unknown } & Record<string, unknown>)[] | null;
}

/** `flow.guide {lang}`: the guide to flows, markdown. */
export interface Guide {
  text: string;
  lang: string;
}

/** An entry of the app's catalog: a bare name, or a name with what it is. */
export type CatalogEntry = string | { name: string; description?: string | null; [key: string]: unknown };

/** `flow.catalog`: what this application registered for definitions to name. */
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

/** The payload of a `flow` entry with `event: "proposal"`, written by `flow_propose`. */
export interface Proposal {
  run: string;
  station: string;
  kind: "rework" | "cancel";
  to?: string | null;
  reason: string;
  /** The session it was written in: where its acceptance is recorded. */
  session?: TaskSession | null;
}

/** A `flow` entry with `event: "proposal_handled"`: a proposal was accepted, once. */
export interface ProposalHandled {
  entry: string;
  action: "rework" | "cancel";
  by: Actor;
}

/** The app-side method names, relative to the module id (`<module>.<name>`). */
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
