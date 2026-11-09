/** The right column for a sub-agent: its figures out of its parent's recording, its task, its transcript. */

import { useEffect, useMemo, useState } from "react";
import { clip, useT } from "@harnix/react";

import type { App, SessionKey } from "../../app.ts";
import { LIVE, SUB_PAGE, SYSTEM } from "../constants.ts";
import { useAgentFacts } from "../data.ts";
import { duration, messageText } from "../format.ts";
import type { HistoryMessage, Task } from "../types.ts";
import { useTopic, type View } from "../view.ts";
import { Facts } from "./facts.tsx";
import { Stats } from "./session.tsx";
import { TaskMessage } from "./tasks.tsx";

/**
 * A sub-agent, picked: its own figures out of its parent's recording, how its task stands, and what
 * it has said — its transcript is a session of its own, read like any.
 */
export function SubAgentOverview({ app, view, session, parent }: { app: App; view: View; session: SessionKey; parent: string }) {
  const tr = useT();
  const parentKey: SessionKey = { instance: session.instance, sessionId: parent };
  const taskParams = useMemo(() => ({ instance: session.instance, session_id: parent }), [session.instance, parent]);
  const listed = useTopic<{ tasks: Task[] }>(view, "harnix.sessions.tasks", taskParams);
  const task = listed?.tasks.find((t) => t.kind === "agent" && t.detail?.agent_session_id === session.sessionId);
  const facts = useAgentFacts(app, parentKey, Boolean(task && LIVE.has(task.status)));
  const own = facts.usage?.find((u) => u.session_id === session.sessionId);
  const [messages, setMessages] = useState<HistoryMessage[] | null>(null);
  // How far back the transcript is read: the last page, and more on asking.
  const [from, setFrom] = useState<number | null>(null);
  const [pages, setPages] = useState(1);
  useEffect(() => {
    let alive = true;
    const target = { instance: session.instance, session_id: session.sessionId, space: SYSTEM };
    void (async () => {
      try {
        const first = await app.call<{ total: number }>("session.history", { ...target, offset: 0, limit: 1 });
        const offset = Math.max(0, first.total - SUB_PAGE * pages);
        const page = await app.call<{ messages: HistoryMessage[] }>("session.history", { ...target, offset, limit: SUB_PAGE * pages });
        if (!alive) return;
        setMessages(page.messages ?? []);
        setFrom(offset);
      } catch {
        if (alive) setMessages([]);
      }
    })();
    return () => {
      alive = false;
    };
  }, [app, session.instance, session.sessionId, task?.status, pages]);
  const d = (task?.detail ?? {}) as Record<string, unknown>;
  return (
    <div className="system-pane" data-testid="system-subagent-pane">
      <div className="agent-head">
        <span className={`dot ${task?.status ?? "done"}`} />
        <span className="nm">{tr("system.task.agent")}</span>
        <span className="kd" data-testid="agent-state">{tr(`system.taskStatus.${task?.status ?? "done"}`)}</span>
      </div>
      <Facts
        rows={[
          [tr("system.fact.session"), session.sessionId],
          [tr("system.fact.parent"), parent],
          [tr("system.fact.agentType"), d.agent_type == null ? "" : String(d.agent_type)],
          [tr("system.fact.what"), d.description == null ? "" : String(d.description)],
          [tr("system.fact.model"), own?.models.join(", ") ?? ""],
          [tr("system.fact.duration"), task ? duration(task) : ""],
        ].filter(([, v]) => v !== "") as [string, string][]}
      />
      <button type="button" className="btn sm outline wide" data-testid="agent-open-parent" onClick={() => view.select({ kind: "session", instance: session.instance, sessionId: parent })}>
        {tr("system.openParent")}
      </button>
      {task && LIVE.has(task.status) ? <TaskMessage app={app} params={taskParams} task={task} /> : null}
      <div className="label">{tr("system.stats")}</div>
      <Stats own={own} turns={[]} />
      <div className="label">{tr("system.messages")}</div>
      <div className="messages" data-testid="subagent-messages">
        {from ? (
          <button type="button" className="btn sm outline wide" data-testid="subagent-earlier" onClick={() => setPages((n) => n + 1)}>
            {tr("system.earlierMessages", { count: from })}
          </button>
        ) : null}
        {messages && !messages.length ? <div className="note">{tr("system.noMessages")}</div> : null}
        {(messages ?? []).map((message) => (
          <div key={message.seq} className="msg" data-role={message.role}>
            <span className="kd">{message.role}</span> {clip(messageText(message.content), 400)}
          </div>
        ))}
      </div>
    </div>
  );
}
