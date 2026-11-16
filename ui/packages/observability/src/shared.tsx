/**
 * What the trajectory and the tabs share: the recording's shapes, which space a call about a session
 * claims, how a call's usage and a message's text are written, how a failure is said, and the ways
 * from a call back to the conversation and from the system space into the session.
 */

import { errorText, type CallSummary, type RecordingRun, type TurnSummary } from "@harnix/core";
import { CONVERSATION_VIEW, compact, t, useT, type HostApi, type HostSessionRef as SessionKey, type Registry, explainError } from "@harnix/react";

/** The view this package registers for the centre: the session's trajectory. */
export const TRAJECTORY_VIEW = "observability.trajectory";

/** The tab's host and the page's registry: what a tab reaches out with. */
export interface Observed {
  host: HostApi;
  registry: Registry;
}

/** One `observability.turns` row: a turn's calls summed (`TurnSummary`, generated from the readers). */
export type TurnRow = TurnSummary;

/** Which run of the session a reading covers: its recording starts over when the process running the session first calls the model. */
export type Run = RecordingRun;

/** One `observability.trace` row: a model call, what went in and what came out (its tool calls' inputs and results are in the call's detail). */
export type TraceCall = CallSummary;

/** Where a call about a session claims it is: the backend space the front space lists (the system space reaches across). */
export function claimFor(r: Registry, host: HostApi): string {
  const front = host.spaces.current();
  return r.at("space").find((c) => c.id === front)?.sessions ?? front;
}

/** The parameters every read of a session carries: the session, and the space claimed for it. */
export function sessionParams({ host, registry }: Observed, session: SessionKey): { instance: string; session_id: string; space: string } {
  return { instance: session.instance, session_id: session.sessionId, space: claimFor(registry, host) };
}

/** What the page says of the run a reading covers: since when, and that an earlier one is gone. */
export function RunNote({ run, testId = "run-note" }: { run?: Run | null; testId?: string }) {
  const tr = useT();
  if (!run) return null;
  const at = new Date(run.created_at);
  const since = Number.isNaN(at.getTime()) ? run.created_at : at.toLocaleString();
  return (
    <div className="note" data-testid={testId} data-earlier={run.earlier_run ? "true" : undefined}>
      {tr("system.runSince", { at: since })}
      {run.earlier_run ? ` ${tr("system.earlierRunGone")}` : ""}
    </div>
  );
}

/** One call's tokens in and out. */
export function usageText(usage: TraceCall["usage"]): string {
  if (!usage) return "—";
  const input = (usage.input_tokens ?? 0) + (usage.cache_read_input_tokens ?? 0) + (usage.cache_creation_input_tokens ?? 0);
  return `in ${compact(input)} · out ${compact(usage.output_tokens ?? 0)}`;
}

export function seconds(ms?: number | null): string {
  return ms != null ? `${(ms / 1000).toFixed(1)}s` : "—";
}

export function messageText(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .map((block: { type?: string; text?: string; content?: unknown; name?: string }) => {
      if (block.type === "text") return block.text ?? "";
      if (block.type === "tool_result") return messageText(block.content);
      if (block.type === "tool_use") return `⚙ ${block.name ?? ""}`;
      if (block.type === "image") return "▣";
      return "";
    })
    .filter(Boolean)
    .join(" ");
}

/** The codes these pages have words for (the system space's words: they are drawn inside it too). */
const SAID: Record<number, string> = {
  [-31000]: "system.error.refused",
  [-31002]: "system.error.noInstance",
  [-31003]: "system.error.space",
  [-31004]: "system.error.unavailable",
  [-31005]: "system.error.tooMany",
  [-32000]: "system.error.noSession",
  [-32015]: "system.error.busy",
  [-32601]: "system.error.noMethod",
};

function codeOf(e: unknown): number | undefined {
  const code = e && typeof e === "object" ? (e as { code?: unknown }).code : undefined;
  return typeof code === "number" ? code : undefined;
}

/** A page's failure: its own words for a code it knows, the backend's (with the code) in the title. */
export function Failure({ error, testId, className = "empty" }: { error: unknown; testId?: string; className?: string }) {
  useT();
  const code = codeOf(error);
  const key = SAID[code ?? 0];
  return (
    <div className={className} data-testid={testId} data-code={code} title={code === undefined ? errorText(error) : `${errorText(error)} (${code})`}>
      {key ? t(key) : errorText(error)}
    </div>
  );
}

/**
 * From a call back to the conversation: the session opened where it belongs, shown as its
 * conversation, and the tool call it asked for (if it asked for one) scrolled to and highlighted.
 */
export async function locateInConversation(host: HostApi, session: SessionKey, tool: string | null): Promise<void> {
  try {
    if (tool) await host.session.open(session, { switchSpace: true, view: CONVERSATION_VIEW, reveal: tool });
    else await host.session.open(session, { switchSpace: true, view: CONVERSATION_VIEW });
  } catch (e) {
    host.shell.notice(explainError(e), "error");
  }
}

/** Into the session's trajectory: the session opened where it belongs (the space it is in brought to the front), shown as its trajectory. */
export async function openTrajectory(host: HostApi, session: SessionKey): Promise<void> {
  try {
    await host.session.open(session, { switchSpace: true, view: TRAJECTORY_VIEW });
  } catch (e) {
    host.shell.notice(explainError(e), "error");
  }
}
