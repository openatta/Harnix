/** What the system space reads: the shapes of the hub's topics and the system module's answers. */

import type { Connection } from "../dialogs.tsx";

export interface InstanceRow {
  id: string;
  carrier: string;
  template?: string | null;
  params?: Record<string, string>;
  owner: string;
  status: { state: string; reason?: string };
  version?: string | null;
  protocol_version?: number | null;
  data_root?: string;
  scenes?: string[];
  pid?: number | null;
  created_ms?: number | null;
}

export interface SessionRowView {
  session_id: string;
  instance?: string;
  name?: string | null;
  scene?: string | null;
  status?: string | null;
  space?: string;
  last_active?: string;
  /** How many of its tasks are running: work that can outlive the turn that started it. */
  tasks?: number;
  running?: boolean;
  queued?: number;
  /** What it waits on a person for: a permission, an answer to the model's question, or a plan's review. */
  waiting?: "approval" | "answer" | "plan" | null;
  /** `harnix`: a module or the kernel started it, not a person. */
  started_by?: "user" | "harnix";
  /** When the running turn began (ms since the epoch). */
  turn_started_ms?: number | null;
  /** Its goal as the hub last heard it; absent when the hub has heard nothing of it. */
  goal?: Goal | null;
  project_root?: string | null;
}

export interface Task {
  task_id: string;
  kind: string;
  status: string;
  started_ms?: number | null;
  ended_ms?: number | null;
  detail?: Record<string, unknown>;
  error?: string;
  usage?: { input_tokens?: number; output_tokens?: number; cache_read_input_tokens?: number; cache_creation_input_tokens?: number };
  waiting_for?: string[];
}

export interface Process {
  app?: { name: string; version: string };
  harnix?: string;
  agix?: string | null;
  uptime_secs?: number;
  peak_rss_bytes?: number | null;
  threads?: number | null;
  connections?: Connection[];
}

export type Scope = "active" | "all";
export type ConsoleTab = Scope | "flows" | "apps" | "diagnostics";

/** A main Agent: a top-level session of an instance, as the instance's session list has it. */
export interface AgentRow extends SessionRowView {
  instance: string;
  /** `instance/session`. */
  key: string;
}

/** What one Agent of a session has used in its current run (`harnix.system.usage`). */
export interface AgentUsage {
  session_id: string;
  parent_session_id?: string | null;
  calls: number;
  turns: number;
  models: string[];
  input_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  output_tokens: number;
  model_ms: number;
  errors: number;
  last_at?: number | null;
}

/** A session's goal (`session.goal`): one aim the engine keeps opening turns for. */
export interface Goal {
  objective: string;
  phase: "active" | "paused" | "blocked" | "complete";
  armed: boolean;
  max_rounds: number;
  rounds_started: number;
  blocked_reason?: { code?: string; message?: string } | string | null;
}

/** One `harnix.system.turns` row: a turn's calls summed. */
export interface TurnRow {
  turn: number;
  calls: number[];
  auxiliary: number;
  duration_ms?: number | null;
  ttft_ms?: number | null;
  model_ms: number;
  tool_ms: number;
  input_tokens: number;
  output_tokens: number;
  cache_hit?: number | null;
  tokens_per_s?: number | null;
  context_tokens?: number | null;
  context_delta?: number | null;
}

export interface TraceCall {
  index: number;
  turn: number;
  step: number;
  purpose?: string | null;
  model: string;
  input: { system: { source?: string | null; chars: number }[]; tools: number; messages: number };
  output: { text: string; chars: number; thinking_chars: number; tool_uses: number };
  tools: { id: string; name: string; input: string; result?: string | null; result_chars?: number | null; is_error?: boolean | null }[];
  outcome: { status: string; error?: string; message?: string };
  stop_reason?: string | null;
  usage?: { input_tokens?: number; output_tokens?: number; cache_read_input_tokens?: number; cache_creation_input_tokens?: number } | null;
  /** When it was sent, how long to its first chunk, how long it streamed; absent without the times. */
  timing?: { sent_at: number; ttft_ms?: number | null; generate_ms?: number | null; tokens_per_s?: number | null } | null;
  duration_ms?: number | null;
}

export interface Context {
  model?: string;
  max_tokens?: number;
  system?: { name: string; origin: unknown; tokens: number; text: string }[];
  tools?: { name: string; source?: string; tokens?: number }[];
  messages?: { count: number; tokens: number };
  total_tokens?: number;
  compact_threshold?: number;
  /** What the provider counted for the last call made; absent before the first. */
  measured?: { tokens: number; window?: number; compact_threshold?: number };
}

export interface HistoryMessage {
  seq: number;
  role: string;
  origin?: string | null;
  content: unknown;
}

/** A session's teams (Agix `team_progress`, as the hub last heard it): each one's stage and members,
 *  every member with how its task stands. */
export interface Team {
  team: string;
  team_id: string;
  stage: string;
  stage_index: number;
  stage_count: number;
  status: string;
  members: string[];
  failed: string[];
}
