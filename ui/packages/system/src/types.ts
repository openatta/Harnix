/**
 * What the system space reads: the shapes of the hub's topics and the system module's answers.
 *
 * The kernel's rows and the readers' figures are `@harnix/core`'s, generated from the Rust that
 * serializes them; the names this package grew up with are kept as aliases. What is declared here
 * is the engine's (`system.*` passes it through) and this package's own.
 */

import type { ReactNode } from "react";
import type { ContextReportData } from "@harnix/react";
import type {
  AgentUsage as ReadAgentUsage,
  CallSummary,
  Goal as EngineGoal,
  HistoryMessage as EngineHistoryMessage,
  InstanceRow as KernelInstanceRow,
  LinkStatus,
  SessionRow,
  SessionUsage,
  TaskRow,
  TurnSummary,
} from "@harnix/core";

/** An instance, as the topic `harnix.instances` lists it. */
export type InstanceRow = KernelInstanceRow;

/** Why a link is as it is, when its state says. */
export function statusReason(status: LinkStatus): string | undefined {
  return "reason" in status ? status.reason : undefined;
}

/** A session's row in `harnix.instances.sessions`. */
export type SessionRowView = SessionRow;

/** A task (`engine.session.tasks`, through the topic `harnix.session.tasks`). */
export type Task = TaskRow;

/** What one Agent of a session has used in its current run (`observability.usage`). */
export type AgentUsage = ReadAgentUsage;

/** Which run the figures of a session cover, and whether the instance was asleep when they were last read (`observability.usage`). */
export type UsageNote = Partial<Pick<SessionUsage, "run" | "stale">>;

/** A session's goal (`engine.session.goal`): one aim the engine keeps opening turns for. */
export type Goal = EngineGoal;

/** One `observability.turns` row: a turn's calls summed. */
export type TurnRow = TurnSummary;

/** One `observability.trace` row. */
export type TraceCall = CallSummary;

/** A message of a session's history (`engine.session.history`), where every page has its place. */
export type HistoryMessage = EngineHistoryMessage & { seq: number };

/** One connection to this process, as `system.process` lists it. */
export interface Connection {
  client: number;
  device: string;
  topology: string;
  user_agent?: string;
  since_ms?: number;
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
/** The console's tab in front: one of the Agents tabs, or a contributed tab's id. */
export type ConsoleTabId = Scope | (string & {});

/** A tab of the console (`system.console.tab`): another package's page among the Agents tabs. */
export interface ConsoleTab {
  id: string;
  title(): string;
  render(): ReactNode;
  /** Where it goes among the contributed tabs; 100 when absent. */
  order?: number;
  /** What the right column says while the tab is in front and nothing is picked. */
  about?(): string;
}

/** What an inspector tab is asked about: a main Agent's session, and a tool call to pick out. */
export interface InspectorTarget {
  kind: "session";
  instance: string;
  sessionId: string;
  /** A tool call's id the person came for (the tool row's "view in the system space"). */
  tool?: string;
}

/** A tab of the right column for a session (`system.inspector.tab`). */
export interface InspectorTab {
  id: string;
  title(target: InspectorTarget): string;
  /** Read at every draw; a throw drops this tab only. */
  match(target: InspectorTarget): boolean;
  render(target: InspectorTarget): ReactNode;
  /** It shows a session's tool calls: the one opened when a person comes from a tool row. */
  tools?: boolean;
}

/** A main Agent: a top-level session of an instance, as the instance's session list has it. */
export interface AgentRow extends SessionRowView {
  instance: string;
  /** `instance/session`. */
  key: string;
}

/** `system.context`'s answer: the shared report's shape (`@harnix/react`'s `ContextReportData`). */
export type Context = ContextReportData;

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

/** A `system.metrics.link` registration: where the cell `metric` of the metrics bar leads. */
export interface MetricsLink {
  id: string;
  /** The cell's id (`data-kpi`): `waiting` is "等你处理". */
  metric: string;
  /** What it opens, for the cell's tooltip. */
  title(): string;
  open(): void | Promise<void>;
}
