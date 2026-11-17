/**
 * The output tab of the right column (`sidebar.tab`, kind `output`): one background Shell command or
 * Monitor of the open session, its output followed read-only as it comes. A tab per task, the task
 * named in the tab's address so that a refresh finds it again; the session's surface keeps it
 * mounted while another tab is in front, so the scrollback stays where it was.
 *
 * The tail is read first (`observability.tasks.output`, the engine's `taskOutput`), then the
 * session's `task_progress` frames are taken as they are pushed; a gap between the two is read
 * again. The task itself comes from the session's task list (`harnix.session.tasks`).
 */

import { useEffect, useRef, useState } from "react";
import type { TaskRow } from "@harnix/core";
import { clip, parseResource, resourceAddress, useT, useTabInfo, type HostApi, type HostSessionRef as SessionKey } from "@harnix/react";

import { sessionParams, type Observed } from "./shared.tsx";

/** The tab type's kind, and the task kinds whose output is a stream of lines. */
export const OUTPUT_KIND = "output";
const STREAMED = new Set(["shell", "monitor"]);
/** The tools that start a task whose output can be followed. */
export const OUTPUT_TOOLS = new Set(["Bash", "Monitor"]);

/** A task's statuses while it is still producing. */
const LIVE = new Set(["running", "waiting", "pending"]);
/** The most of a task's output this page keeps: the engine keeps the last 256 KiB. */
const OUTPUT_KEPT = 256 * 1024;
/** How far from the end (px) still counts as at the end. */
const TAIL_SLACK = 8;

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
export function hasOutput(task: Pick<TaskRow, "kind">): boolean {
  return STREAMED.has(task.kind);
}

/** What a task is, in a few words: its description, command, prompt or call; its id when it says nothing. */
export function taskLabel(task: TaskRow): string {
  const d = task.detail ?? {};
  // An MCP task has no description of its own: it is the call of one server's tool.
  const call = typeof d.tool === "string" && d.tool ? `${typeof d.server === "string" ? d.server : ""} ${d.tool}`.trim() : undefined;
  const text = d.description ?? d.command ?? d.prompt ?? d.member ?? d.cron ?? call ?? task.task_id;
  return clip(String(text), 120);
}

/** Terminal escape sequences: colours and cursor moves (CSI), and titles or links (OSC). */
const ESCAPES = /\u001b\[[0-?]*[ -/]*[@-~]|\u001b\][^\u0007\u001b]*(?:\u0007|\u001b\\)|\u001b[@-Z\\-_]/g;

/** The text of a command's output without its colour codes, which a page cannot draw. */
export function stripAnsi(text: string): string {
  return text.includes("\u001b") ? text.replace(ESCAPES, "") : text;
}

/**
 * Open the output of the task a tool call started: the session's task whose `detail.tool_use_id`
 * is the call's. Resolves to false when there is none — a foreground command has no task.
 */
export async function openToolOutput(host: HostApi, session: SessionKey, toolUseId: string): Promise<boolean> {
  const { tasks } = await host.instances.tasks(session.instance, session.sessionId);
  const task = tasks.find((x) => hasOutput(x) && x.detail?.tool_use_id === toolUseId);
  if (!task) return false;
  openTaskOutput(host, session, task);
  return true;
}

/** Open (or add) the output tab of a task of `session`, the open session. */
export function openTaskOutput(host: HostApi, session: SessionKey, task: TaskRow): void {
  labels.set(task.task_id, taskLabel(task));
  host.sidebar.openResource(outputAddress(session, task.task_id), { kind: OUTPUT_KIND, params: { task: task.task_id } });
}

/** A hub topic's latest value, followed while drawn. */
function useTopic<T>(host: HostApi, topic: string, params: Record<string, unknown>): T | undefined {
  const [value, setValue] = useState<T | undefined>(undefined);
  const key = JSON.stringify(params);
  useEffect(() => host.subscribe(topic, params, (v) => setValue(v as T)), [host, topic, key]);
  return value;
}

export function OutputTab({ observed }: { observed: Observed }) {
  const tr = useT();
  const { tab } = useTabInfo();
  const host = observed.host;
  const named = parseOutputAddress(tab.address);
  const session = named?.session ?? { instance: "", sessionId: "" };
  const listed = useTopic<{ tasks: TaskRow[] }>(host, "harnix.session.tasks", { instance: session.instance, session_id: session.sessionId });
  if (!named) return <div className="note">{tr("obs.output.none")}</div>;
  const task = listed?.tasks.find((x) => x.task_id === named.taskId);
  if (task) labels.set(task.task_id, taskLabel(task));
  if (!listed) return <div className="note">{tr("obs.output.loading")}</div>;
  if (!task) return <div className="note" data-testid="output-gone">{tr("obs.output.gone")}</div>;
  return (
    <div className="obs-output output-tab" data-testid="output-tab" data-task={task.task_id} data-status={task.status}>
      <div className="output-head">
        <span className={`dot ${task.status}`} />
        <span className="what truncate" title={taskLabel(task)}>
          {taskLabel(task)}
        </span>
        <span className="when" data-testid="output-status">
          {tr(`obs.taskStatus.${task.status}`)}
        </span>
      </div>
      {task.error ? <div className="hint err">{task.error}</div> : null}
      <TaskOutput observed={observed} session={session} task={task} />
    </div>
  );
}

/** One read of a task's output (`engine.session.taskOutput`): the bytes from `from_byte` to `next_byte`. */
interface OutputSlice {
  output: string;
  from_byte: number;
  next_byte: number;
  dropped: number;
  final: boolean;
}

/** The part of `text` from byte `from` on: offsets are bytes of UTF-8, not characters. */
function fromByte(text: string, from: number): string {
  if (from <= 0) return text;
  return new TextDecoder().decode(new TextEncoder().encode(text).subarray(from));
}

/**
 * What a task has produced so far: a shell command's or a monitor's lines as they come. Followed as
 * the engine's protocol has it: the tail is read first, then the session's `task_progress` frames
 * are taken as they arrive; a gap (bytes skipped, a frame missed) is read again from where the page
 * is. Following keeps the newest in view; reading back stops it, reaching the end again resumes it.
 */
export function TaskOutput({ observed, session, task }: { observed: Observed; session: SessionKey; task: TaskRow }) {
  const tr = useT();
  const host = observed.host;
  const [out, setOut] = useState<{ text: string; earlier: boolean; final: boolean } | null>(null);
  const [follow, setFollow] = useState(true);
  const pre = useRef<HTMLPreElement>(null);
  const live = LIVE.has(task.status);
  const target = sessionParams(observed, session);
  const targetKey = JSON.stringify(target);
  // Following: what just arrived stays in view.
  useEffect(() => {
    if (follow && pre.current) pre.current.scrollTop = pre.current.scrollHeight;
  }, [out, follow]);
  // Reading back stops the following; reaching the end again resumes it.
  const scrolled = () => {
    const el = pre.current;
    if (!el) return;
    const atEnd = el.scrollHeight - el.scrollTop - el.clientHeight <= TAIL_SLACK;
    setFollow((was) => (was === atEnd ? was : atEnd));
  };
  useEffect(() => {
    let stopped = false;
    let next: number | null = null;
    const add = (text: string, gap: number, final: boolean) =>
      setOut((now) => {
        if (!now) return { text, earlier: false, final };
        const joined = now.text + (gap > 0 ? `\n${tr("obs.outputDropped", { count: gap })}\n` : "") + text;
        return { text: joined.length > OUTPUT_KEPT ? joined.slice(-OUTPUT_KEPT) : joined, earlier: now.earlier || joined.length > OUTPUT_KEPT, final: final || now.final };
      });
    const read = async () => {
      try {
        const at = next;
        const r = await host.call<OutputSlice>("observability.tasks.output", { ...target, task_id: task.task_id, ...(at === null ? {} : { from_byte: at }) });
        if (stopped) return;
        if (next !== null && r.next_byte <= next) return; // a frame got there first
        next = r.next_byte;
        if (at === null) setOut({ text: r.output, earlier: r.from_byte > 0, final: r.final });
        else add(r.output, r.dropped, r.final);
      } catch {
        if (!stopped) setOut(null);
      }
    };
    const off = host.events.onSessionEvent(session, (e) => {
      if (stopped) return;
      const event = (e.raw ?? {}) as { kind?: string; task_id?: string; task?: { task_id?: string }; appended?: string; from_byte?: number; next_byte?: number };
      if (event.kind === "task_progress" && event.task_id === task.task_id && next !== null) {
        const from = event.from_byte ?? 0;
        const to = event.next_byte ?? 0;
        if (to <= next) return; // already here
        if (from <= next) {
          add(fromByte(event.appended ?? "", next - from), 0, false);
          next = to;
        } else {
          void read(); // a gap: skipped bytes or a missed frame
        }
      }
      if (event.kind === "task_finished" && event.task?.task_id === task.task_id) void read();
    });
    // Attached, so the frames come while this tab is drawn; let go only when the session is not the open one.
    void host.call("harnix.session.attach", target).catch(() => undefined).then(() => read());
    return () => {
      stopped = true;
      off();
      if (!host.session.isOpen(session)) void host.call("harnix.session.detach", target).catch(() => undefined);
    };
    // `targetKey` stands for `target` and `session`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [host, targetKey, task.task_id]);
  if (!out) return null;
  return (
    <div className="task-output" data-testid="task-output" data-final={out.final ? "" : undefined}>
      <div className="label">
        {tr("obs.taskOutput")}
        {out.earlier ? <span className="hint">{tr("obs.outputTruncated")}</span> : null}
        {live ? (
          <label className="check end">
            <input type="checkbox" data-testid="task-follow" checked={follow} onChange={(e) => setFollow(e.target.checked)} />
            {tr("obs.follow")}
          </label>
        ) : null}
      </div>
      <pre className="hx-pre scroll" ref={pre} data-testid="task-output-text" onScroll={scrolled}>
        {stripAnsi(out.text) || tr("obs.noOutput")}
      </pre>
    </div>
  );
}
