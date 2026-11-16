/**
 * The output tab of the right column (`sidebar.tab`, kind `output`): one background Shell command or
 * Monitor of the open session, its output followed read-only as it comes. A tab per task, the task
 * named in the tab's address so that a refresh finds it again; the session's surface keeps it
 * mounted while another tab is in front, so the scrollback stays where it was.
 */

import { parseResource, resourceAddress, useT, useTabInfo, type HostApi, type HostSessionRef as SessionKey } from "@harnix/react";

import { taskLabel } from "../format.ts";
import type { Task } from "../types.ts";
import { useTopic, type View } from "../view.ts";
import { TaskOutput } from "./tasks.tsx";

/** The tab type's kind, and the task kinds whose output is a stream of lines. */
export const OUTPUT_KIND = "output";
const STREAMED = new Set(["shell", "monitor"]);

/** What a title can say of a task before its tab has read the list: the label the opener knew. */
const labels = new Map<string, string>();

/** The address of one task's output tab: the session and the task, each URL-encoded. */
export function outputAddress(session: SessionKey, taskId: string): string {
  return resourceAddress(OUTPUT_KIND, [session.instance, session.sessionId, taskId].map(encodeURIComponent).join("/"));
}

/** The session and task an output tab's address names; null for one that names none. */
export function parseOutputAddress(address: string | null | undefined): { session: SessionKey; taskId: string } | null {
  const parsed = address ? parseResource(address) : null;
  if (!parsed || parsed.type !== OUTPUT_KIND) return null;
  const parts = parsed.path.split("/");
  if (parts.length !== 3 || parts.some((p) => !p)) return null;
  try {
    const [instance, sessionId, taskId] = parts.map(decodeURIComponent) as [string, string, string];
    return { session: { instance, sessionId }, taskId };
  } catch {
    return null;
  }
}

/** The tab's title: the task's own words once known, else its id. */
export function outputTitle(address: string | undefined, fallback: string): string {
  const named = parseOutputAddress(address);
  return named ? (labels.get(named.taskId) ?? fallback) : fallback;
}

/** Whether a task has a stream of output to follow. */
export function hasOutput(task: Pick<Task, "kind">): boolean {
  return STREAMED.has(task.kind);
}

/**
 * Open the output of the task a tool call started: the session's task whose `detail.tool_use_id`
 * is the call's. Resolves to false when there is none — a foreground command has no task.
 */
export async function openToolOutput(host: HostApi, session: SessionKey, toolUseId: string): Promise<boolean> {
  const { tasks } = await host.instances.tasks(session.instance, session.sessionId);
  const task = (tasks as Task[]).find((x) => hasOutput(x) && x.detail?.tool_use_id === toolUseId);
  if (!task) return false;
  openTaskOutput(host, session, task);
  return true;
}

/** Open (or add) the output tab of a task of `session`, the open session. */
export function openTaskOutput(host: HostApi, session: SessionKey, task: Task): void {
  labels.set(task.task_id, taskLabel(task));
  host.sidebar.openResource(outputAddress(session, task.task_id), { kind: OUTPUT_KIND, params: { task: task.task_id } });
}

export function OutputTab({ view }: { view: View }) {
  const tr = useT();
  const { tab } = useTabInfo();
  const named = parseOutputAddress(tab.address);
  const session = named?.session ?? { instance: "", sessionId: "" };
  const listed = useTopic<{ tasks: Task[] }>(view, "harnix.session.tasks", { instance: session.instance, session_id: session.sessionId });
  if (!named) return <div className="note">{tr("system.output.none")}</div>;
  const task = listed?.tasks.find((x) => x.task_id === named.taskId);
  if (task) labels.set(task.task_id, taskLabel(task));
  if (!listed) return <div className="note">{tr("system.output.loading")}</div>;
  if (!task) return <div className="note" data-testid="output-gone">{tr("system.output.gone")}</div>;
  const params = { instance: session.instance, session_id: session.sessionId };
  return (
    <div className="system-pane output-tab" data-testid="output-tab" data-task={task.task_id} data-status={task.status}>
      <div className="output-head">
        <span className={`dot ${task.status}`} />
        <span className="what truncate" title={taskLabel(task)}>
          {taskLabel(task)}
        </span>
        <span className="when" data-testid="output-status">
          {tr(`system.taskStatus.${task.status}`)}
        </span>
      </div>
      {task.error ? <div className="hint err">{task.error}</div> : null}
      <TaskOutput host={view.host} params={params} task={task} />
    </div>
  );
}
