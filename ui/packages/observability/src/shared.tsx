/**
 * What the calls and turns tabs share: the recording's shapes, how a call's usage and a message's
 * text are written, how a failure is said, and the way back to the conversation.
 */

import { errorText, type CallSummary, type RecordingRun, type TurnSummary } from "@harnix/core";
import { compact, t, useT, type HostApi, type HostSessionRef as SessionKey, type Registry } from "@harnix/react";

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

/** The codes these tabs have words for (the system space's words: they are drawn inside it). */
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

/** A tab's failure: the page's own words for a code it knows, the backend's (with the code) in the title. */
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
 * From a call back to the conversation: the session opened in the interface space that lists its
 * backend space, and the tool call it asked for (if it asked for one) scrolled to and flashed. The
 * row is drawn once the history is in, so it is looked for a few times.
 */
export async function locateInConversation({ host, registry }: Observed, session: SessionKey, tool: string | null): Promise<void> {
  const listed = await host.call<{ sessions: { session_id: string; instance?: string; space?: string }[] }>("harnix.sessions.list", { instance: session.instance }).catch(() => ({ sessions: [] }));
  const backend = listed.sessions.find((r) => r.session_id === session.sessionId)?.space ?? host.defaults.space;
  if (!backend) return;
  const front = host.spaces.current();
  const space = registry.at("space").find((c) => (c.sessions ?? c.id) === backend && c.id !== front)?.id ?? backend;
  try {
    await host.navigate.open({ space, session });
  } catch (e) {
    host.shell.notice(errorText(e), "error");
    return;
  }
  if (!tool) return;
  for (let i = 0; i < 25; i += 1) {
    await new Promise((resolve) => setTimeout(resolve, 200));
    const node = document.querySelector<HTMLElement>(`[data-testid="conv-tool"][data-id="${CSS.escape(tool)}"]`);
    if (!node) continue;
    node.scrollIntoView({ block: "center" });
    node.classList.add("hx-flash");
    node.dataset.located = "true";
    setTimeout(() => node.classList.remove("hx-flash"), 2000);
    return;
  }
}
