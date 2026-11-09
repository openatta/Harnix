/** The tasks tab: a session's tasks and teams, a task's detail, its output followed as it comes. */

import { useEffect, useMemo, useRef, useState } from "react";
import { Disclosure, useT } from "@harnix/react";

import { keyOf, type App, type SessionKey } from "../../app.ts";
import { LIVE, OUTPUT_KEPT, STOP_ARM_MS, SYSTEM } from "../constants.ts";
import { failureText } from "../failure.tsx";
import { taskDuration, taskLabel } from "../format.ts";
import type { Task, Team } from "../types.ts";
import { act, useSelection, useTopic, type View } from "../view.ts";
import { Facts } from "./facts.tsx";

export function TasksTab({ app, view, session }: { app: App; view: View; session: SessionKey }) {
  const tr = useT();
  const picked = useSelection(view);
  const params = useMemo(() => ({ instance: session.instance, session_id: session.sessionId }), [session.instance, session.sessionId]);
  const listed = useTopic<{ tasks: Task[]; teams?: Team[] }>(view, "harnix.sessions.tasks", params);
  const tasks = listed?.tasks ?? [];
  if (!tasks.length && !listed?.teams?.length) return <div className="note">{tr("system.noTasks")}</div>;
  const chosen = picked?.kind === "task" ? picked.taskId : null;
  return (
    <div className="system-pane tasks" data-testid="system-tasks">
      <TeamBoard teams={listed?.teams ?? []} tasks={tasks} />
      {tasks.map((task) => (
        <div key={task.task_id} className={`task${task.task_id === chosen ? " on" : ""}`} data-kind={task.kind} data-status={task.status}>
          <div className="task-h">
            {/* The task is picked by its own button; stopping it is the one beside, not inside. */}
            <button
              type="button"
              className="task-pick"
              data-testid="task-pick"
              aria-expanded={task.task_id === chosen}
              onClick={() => view.select({ kind: "task", instance: session.instance, sessionId: session.sessionId, taskId: task.task_id })}
            >
              <span className={`dot ${task.status}`} />
              <span className="kind">{tr(`system.task.${task.kind}`)}</span>
              <span className="what truncate" title={taskLabel(task)}>
                {taskLabel(task)}
              </span>
              <span className="when">{taskDuration(task)}</span>
            </button>
            {LIVE.has(task.status) ? (
              <StopTask onStop={() => void act(app, () => app.call("harnix.system.tasks.stop", { ...params, task_id: task.task_id }))} />
            ) : null}
          </div>
          {task.task_id === chosen ? <TaskDetail app={app} view={view} params={params} task={task} /> : null}
        </div>
      ))}
    </div>
  );
}

/**
 * A message for a sub-agent or teammate that is running (Agix `session.messageTask`): it reaches the
 * turn it is in at its next step, as a message sent mid-turn reaches a main Agent. Not delivered —
 * it has finished, or is between turns — is said, not hidden.
 */
export function TaskMessage({ app, params, task }: { app: App; params: { instance: string; session_id: string }; task: Task }) {
  const tr = useT();
  const [text, setText] = useState("");
  const [said, setSaid] = useState<{ ok: boolean; text: string } | null>(null);
  const send = async () => {
    const message = text.trim();
    if (!message) return;
    try {
      const r = await app.call<{ delivered: boolean; reason?: string }>("harnix.system.tasks.message", { ...params, task_id: task.task_id, message });
      setSaid(r.delivered ? { ok: true, text: tr("system.message.delivered") } : { ok: false, text: tr("system.message.notDelivered", { reason: r.reason ?? "" }) });
      if (r.delivered) setText("");
    } catch (e) {
      setSaid({ ok: false, text: failureText(e) });
    }
  };
  return (
    <div className="task-message" data-testid="task-message">
      <div className="label">{tr("system.message.title")}</div>
      <textarea
        data-testid="task-message-text"
        rows={2}
        value={text}
        placeholder={tr("system.message.placeholder")}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            void send();
          }
        }}
      />
      <button type="button" className="btn sm primary" data-testid="task-message-send" disabled={!text.trim()} onClick={() => void send()}>
        {tr("system.message.send")}
      </button>
      {said ? (
        <div className={`hint${said.ok ? "" : " err"}`} data-testid="task-message-said">
          {said.text}
        </div>
      ) : null}
    </div>
  );
}

/** The task states whose count is said as one phrase (`system.taskCount.*`): "3 running", "3 个运行中". */
const COUNTED = new Set(["running", "waiting", "pending", "done", "failed", "stopped"]);

export function TeamBoard({ teams, tasks }: { teams: Team[]; tasks: Task[] }) {
  const tr = useT();
  if (!teams.length) return null;
  return (
    <div className="teams" data-testid="team-board">
      {teams.map((team) => {
        const mine = tasks.filter((task) => task.kind === "teammate" && task.detail?.team === team.team);
        // A member's own task says how it stands when there is one (a standing teammate); otherwise the
        // stage does: dispatched is running, finished is done, unless the member is among the failed.
        const statusOf = (member: string) =>
          team.failed.includes(member)
            ? "failed"
            : mine.find((task) => task.detail?.member === member)?.status ?? (team.status === "completed" ? "done" : "running");
        const counts = team.members.reduce<Record<string, number>>((n, member) => ({ ...n, [statusOf(member)]: (n[statusOf(member)] ?? 0) + 1 }), {});
        return (
          <div key={team.team_id} className="team" data-team={team.team} data-status={team.status}>
            <div className="team-h">
              <span className="nm">{team.team}</span>
              <span className="kd">{tr("system.team.stage", { index: team.stage_index + 1, count: team.stage_count, stage: team.stage })}</span>
              <span className={`kd phase ${team.status}`}>{team.status}</span>
              <span className="note">{Object.entries(counts).map(([status, n]) => (COUNTED.has(status) ? tr(`system.taskCount.${status}`, { n }) : `${tr(`system.taskStatus.${status}`)} ${n}`)).join(" · ")}</span>
            </div>
            {team.members.map((member) => (
              <div key={member} className="member" data-member={member} data-status={statusOf(member)}>
                <span className={`dot ${statusOf(member)}`} />
                <span>{member}</span>
                <span className="note">{tr(`system.taskStatus.${statusOf(member)}`)}</span>
              </div>
            ))}
          </div>
        );
      })}
    </div>
  );
}

/** Stop, as DSH has it: the first press arms the button, a second within three seconds stops. */
function StopTask({ onStop }: { onStop: () => void }) {
  const tr = useT();
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    if (!armed) return;
    const timer = setTimeout(() => setArmed(false), STOP_ARM_MS);
    return () => clearTimeout(timer);
  }, [armed]);
  return (
    <button
      type="button"
      className={`btn sm outline${armed ? " danger" : ""}`}
      data-testid="task-stop"
      data-armed={armed ? "" : undefined}
      onClick={(e) => {
        e.stopPropagation();
        if (!armed) return setArmed(true);
        setArmed(false);
        onStop();
      }}
    >
      {tr(armed ? "system.stopConfirm" : "system.stop")}
    </button>
  );
}

/** One read of a task's output (`session.taskOutput`): the bytes from `from_byte` to `next_byte`. */
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
 * What a task has produced so far: a shell command's or a monitor's lines as they come, a
 * sub-agent's answer once it has one. Followed as Agix's protocol has it (§6.3): the tail is read
 * first (`session.taskOutput`), then the session's `task_progress` frames are taken as they are
 * pushed — what a frame repeats is dropped, a frame past a gap sends one read from where this page
 * is, and that read says so when the engine had already let some go. `task_finished` brings a last
 * read, after which the output is final. The session is attached claiming the system space for as
 * long as this is drawn, and detached after unless the page has it open elsewhere.
 */
function TaskOutput({ app, params, task }: { app: App; params: { instance: string; session_id: string }; task: Task }) {
  const tr = useT();
  const [out, setOut] = useState<{ text: string; earlier: boolean; final: boolean } | null>(null);
  const [follow, setFollow] = useState(true);
  const pre = useRef<HTMLPreElement>(null);
  const live = LIVE.has(task.status);
  // Following: what just arrived stays in view.
  useEffect(() => {
    if (follow && pre.current) pre.current.scrollTop = pre.current.scrollHeight;
  }, [out, follow]);
  useEffect(() => {
    let stopped = false;
    let next: number | null = null;
    const add = (text: string, gap: number, final: boolean) =>
      setOut((now) => {
        if (!now) return { text, earlier: false, final };
        const joined = now.text + (gap > 0 ? `\n${tr("system.outputDropped", { count: gap })}\n` : "") + text;
        return { text: joined.length > OUTPUT_KEPT ? joined.slice(-OUTPUT_KEPT) : joined, earlier: now.earlier || joined.length > OUTPUT_KEPT, final: final || now.final };
      });
    const read = async () => {
      try {
        const at = next;
        const r = await app.call<OutputSlice>("harnix.system.tasks.output", { ...params, task_id: task.task_id, ...(at === null ? {} : { from_byte: at }) });
        if (stopped) return;
        if (next !== null && r.next_byte <= next) return; // a frame got there first
        next = r.next_byte;
        if (at === null) setOut({ text: r.output, earlier: r.from_byte > 0, final: r.final });
        else add(r.output, r.dropped, r.final);
      } catch {
        if (!stopped) setOut(null);
      }
    };
    const target = { instance: params.instance, session_id: params.session_id, space: SYSTEM };
    const off = app.client.onNotify((method, p) => {
      if (stopped || method !== "harnix.event" || p.session_id !== params.session_id) return;
      const event = (p.event ?? {}) as { kind?: string; task_id?: string; task?: { task_id?: string }; appended?: string; from_byte?: number; next_byte?: number };
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
    void app.call("harnix.attach", target).catch(() => undefined).then(() => read());
    return () => {
      stopped = true;
      off();
      const open = Object.values(app.snapshot.currentBySpace).some((k) => k && keyOf(k.instance, k.sessionId) === keyOf(params.instance, params.session_id));
      if (!open) void app.call("harnix.detach", target).catch(() => undefined);
    };
  }, [app, params, task.task_id]);
  if (!out) return null;
  return (
    <div className="task-output" data-testid="task-output" data-final={out.final ? "" : undefined}>
      <div className="label">
        {tr("system.taskOutput")}
        {out.earlier ? <span className="hint">{tr("system.outputTruncated")}</span> : null}
        {live ? (
          <label className="check end">
            <input type="checkbox" data-testid="task-follow" checked={follow} onChange={(e) => setFollow(e.target.checked)} />
            {tr("system.follow")}
          </label>
        ) : null}
      </div>
      <pre className="hx-pre scroll" ref={pre} data-testid="task-output-text">
        {out.text || tr("system.noOutput")}
      </pre>
    </div>
  );
}

/** A task, picked: what it is, how it stands, what it has used and is waiting on, and its output. */
function TaskDetail({ app, view, params, task }: { app: App; view: View; params: { instance: string; session_id: string }; task: Task }) {
  const tr = useT();
  const d = (task.detail ?? {}) as Record<string, unknown>;
  const usage = task.usage;
  const waiting = task.waiting_for ?? [];
  const when = (ms?: number | null) => (ms ? new Date(ms).toLocaleTimeString() : "");
  const what = d.command ?? d.description ?? d.prompt ?? d.member ?? d.cron;
  const sub = task.kind === "agent" ? (d.agent_session_id as string | undefined) : undefined;
  const rows: [string, string][] = [
    [tr("system.fact.kind"), tr(`system.task.${task.kind}`)],
    [tr("system.fact.status"), tr(`system.taskStatus.${task.status}`)],
    [tr("system.fact.what"), what == null ? "" : String(what)],
    [tr("system.fact.agentType"), d.agent_type == null ? "" : String(d.agent_type)],
    [tr("system.fact.started"), when(task.started_ms)],
    [tr("system.fact.ended"), when(task.ended_ms)],
    [tr("system.fact.duration"), taskDuration(task)],
  ];
  if (usage) {
    rows.push([
      tr("system.fact.tokens"),
      tr("system.tokenTip", {
        input: usage.input_tokens ?? 0,
        read: usage.cache_read_input_tokens ?? 0,
        write: usage.cache_creation_input_tokens ?? 0,
        output: usage.output_tokens ?? 0,
      }),
    ]);
  }
  if (waiting.length) rows.push([tr("system.fact.waitingFor"), waiting.join(", ")]);
  return (
    <div className="task-detail" data-testid="system-task">
      {task.error ? <div className="hint err">{task.error}</div> : null}
      <Facts rows={rows.filter(([, v]) => v !== "")} />
      {sub ? (
        <button
          type="button"
          className="btn sm outline wide"
          data-testid="task-open-agent"
          onClick={() => view.select({ kind: "session", instance: params.instance, sessionId: sub, parent: params.session_id })}
        >
          {tr("system.openSubAgent")}
        </button>
      ) : null}
      {(task.kind === "agent" || task.kind === "teammate") && LIVE.has(task.status) ? <TaskMessage app={app} params={params} task={task} /> : null}
      <Disclosure testId="task-raw" state="ok" summary={<span className="hx-tool-name">{tr("system.rawDetail")}</span>}>
        <pre className="hx-pre">{JSON.stringify(task.detail ?? {}, null, 2)}</pre>
      </Disclosure>
      {task.kind === "cron" ? null : <TaskOutput app={app} params={params} task={task} />}
    </div>
  );
}
