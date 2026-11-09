/** The console's Agents: each main Agent on a row, opened into its sub-agents and tasks. */

import { Fragment, useMemo, useState } from "react";
import { clip, useT } from "@harnix/react";

import type { App } from "../app.ts";
import { compact, millis, since } from "../format.ts";
import { Glyph } from "../glyph.tsx";
import { useShell } from "../hooks.ts";
import { PAGE, SYSTEM } from "./constants.ts";
import { isActive, stateOf, useAgentRows, useGoals, useUsage } from "./data.ts";
import { taskDuration, taskLabel, tokenTip, totalTokens } from "./format.ts";
import { TeamBoard } from "./pane/tasks.tsx";
import type { AgentRow, AgentUsage, Goal, InstanceRow, Scope, Task, Team } from "./types.ts";
import { isSelected, useNow, useSelection, useTopic, useTopicIf, type View } from "./view.ts";

/**
 * The Agents: each main Agent on a row — how it stands, whether Harnix itself started it, and its
 * sub-agents, tasks, tokens and model time — and, opened, its sub-agents as a tree and its tasks.
 * The active tab lists what is doing something or is loaded; the all tab every main Agent of every
 * instance, Harnix's own among them.
 */
export function Agents({ app, view, scope }: { app: App; view: View; scope: Scope }) {
  const tr = useT();
  const listed = useTopic<{ instances: InstanceRow[] }>(view, "harnix.instances");
  const s = useShell(app);
  const turns = new Set(Object.keys(s.running));
  const rows = useAgentRows(view, listed?.instances ?? []);
  const [limit, setLimit] = useState(PAGE);
  const [query, setQuery] = useState("");
  const [who, setWho] = useState<"all" | "user" | "harnix">("all");
  const q = query.trim().toLowerCase();
  const matches = (row: AgentRow) =>
    (who === "all" || (row.started_by ?? "user") === who) &&
    (!q || [row.name, row.session_id, row.space, row.instance, row.scene, row.project_root].some((v) => v && String(v).toLowerCase().includes(q)));
  const inScope = scope === "active" ? rows.filter((row) => isActive(row, turns)) : rows;
  const chosen = inScope.filter(matches);
  const rank = (row: AgentRow) => stateOf(row, turns, view.isUnread(row.key)).rank;
  const sorted = [...chosen].sort((a, b) => rank(a) - rank(b) || (b.last_active ?? "").localeCompare(a.last_active ?? ""));
  // A running turn's clock goes on by the second.
  useNow(sorted.some((row) => row.turn_started_ms) ? 1000 : null);
  const shown = scope === "all" ? sorted.slice(0, limit) : sorted;
  const usage = useUsage(app, shown);
  // A goal is the engine's, in memory: asked of the loaded Agents on the active tab.
  const goals = useGoals(app, shown);
  return (
    <div className="tree2 agents" data-testid="system-agents" data-scope={scope}>
      <div className="agents-bar">
        <input
          type="search"
          className="search"
          data-testid="system-agents-search"
          placeholder={tr("system.agents.search")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <select data-testid="system-agents-who" value={who} onChange={(e) => setWho(e.target.value as typeof who)}>
          <option value="all">{tr("system.agents.who.all")}</option>
          <option value="user">{tr("system.agents.who.user")}</option>
          <option value="harnix">{tr("system.agents.who.harnix")}</option>
        </select>
        <span className="count" data-testid="system-agents-count">{tr("system.agents.count", { count: chosen.length, of: inScope.length })}</span>
      </div>
      {shown.length ? (
        shown.map((row) => <AgentItem key={row.key} app={app} view={view} row={row} usage={usage[row.key]} goal={goals[row.key] ?? null} turns={turns} scope={scope} />)
      ) : (
        <div className="empty">{tr(q || who !== "all" ? "system.agents.noMatch" : scope === "active" ? "system.agents.noneActive" : "system.agents.none")}</div>
      )}
      {sorted.length > shown.length ? (
        <button type="button" className="more" data-testid="system-agents-more" onClick={() => setLimit((n) => n + PAGE)}>
          {tr("system.agents.more", { count: sorted.length - shown.length })}
        </button>
      ) : null}
      <div className="note">{tr(scope === "active" ? "system.agents.activeNote" : "system.agents.allNote")}</div>
    </div>
  );
}

/** A sub-agent: its session, what it has used, and the task that runs it while the engine has one. */
interface SubAgent {
  id: string;
  parent: string;
  usage?: AgentUsage;
  task?: Task;
}

/** The sub-agents under each parent: from the recording, which has every one this run, and from the
 *  main Agent's tasks, which have the ones still known to the engine and how they stand. */
function subAgentsOf(root: string, usage: AgentUsage[] | undefined, tasks: Task[]): Map<string, SubAgent[]> {
  const all = new Map<string, SubAgent>();
  for (const u of usage ?? []) if (u.session_id !== root) all.set(u.session_id, { id: u.session_id, parent: u.parent_session_id ?? root, usage: u });
  for (const task of tasks) {
    const id = task.kind === "agent" ? (task.detail?.agent_session_id as string | undefined) : undefined;
    if (!id) continue;
    const known = all.get(id);
    if (known) known.task = task;
    else all.set(id, { id, parent: root, task });
  }
  const under = new Map<string, SubAgent[]>();
  for (const sub of all.values()) under.set(sub.parent, [...(under.get(sub.parent) ?? []), sub]);
  return under;
}

function AgentItem({ app, view, row, usage, goal, turns, scope }: { app: App; view: View; row: AgentRow; usage?: AgentUsage[]; goal: Goal | null; turns: Set<string>; scope: Scope }) {
  const tr = useT();
  const picked = useSelection(view);
  const state = stateOf(row, turns, view.isUnread(row.key));
  const [open, setOpen] = useState<boolean | null>(null);
  const live = row.tasks ?? 0;
  const expanded = open ?? (scope === "active" && live > 0);
  const taskParams = useMemo(() => ({ instance: row.instance, session_id: row.session_id }), [row.instance, row.session_id]);
  const listed = useTopicIf<{ tasks: Task[]; teams?: Team[] }>(view, "harnix.sessions.tasks", taskParams, expanded || (scope === "active" && row.status === "active"));
  const tasks = listed?.tasks ?? [];
  const subs = subAgentsOf(row.session_id, usage, tasks);
  const subCount = [...subs.values()].reduce((n, list) => n + list.length, 0);
  const own = tasks.filter((task) => task.kind !== "agent");
  const taskCount = listed ? tasks.length : live;
  const tokens = (usage ?? []).reduce((n, u) => n + totalTokens(u), 0);
  const time = (usage ?? []).reduce((n, u) => n + u.model_ms, 0);
  const select = () => view.select({ kind: "session", instance: row.instance, sessionId: row.session_id });
  return (
    <div className={`tgroup${expanded ? " open" : ""}`} data-sid={row.session_id} data-instance={row.instance} data-state={state.dot}>
      <div
        className={`tr agent${isSelected(picked, { kind: "session", instance: row.instance, sessionId: row.session_id }) ? " on" : ""}`}
        data-testid="system-agent"
        title={[row.name || tr("sidebar.untitled"), state.label, `${row.instance} · ${row.space ?? "sessions"} · ${row.scene ?? ""}`, row.project_root ?? "", row.session_id, row.last_active ?? ""].filter(Boolean).join("\n")}
        onClick={select}
      >
        <button
          type="button"
          className="twist"
          data-testid="system-agent-toggle"
          aria-expanded={expanded}
          aria-label={tr(expanded ? "system.collapse" : "system.expand")}
          onClick={(e) => {
            e.stopPropagation();
            setOpen(!expanded);
          }}
        >
          <Glyph className="ic" name={expanded ? "chevronDown" : "chevronRight"} />
        </button>
        <span className={`dot ${state.dot}`} title={state.label} />
        <span className="nm">{row.name || tr("sidebar.untitled")}</span>
        {row.started_by === "harnix" ? <span className="kd own" data-testid="system-agent-harnix" title={tr("system.agents.harnixTip")}>Harnix</span> : null}
        <span className="kd">{row.space ?? "sessions"}</span>
        {row.instance !== SYSTEM ? <span className="kd">{row.instance}</span> : null}
        {goal && goal.phase !== "complete" ? (
          <span className={`kd goal ${goal.phase}`} data-testid="system-agent-goal" data-phase={goal.phase} title={goal.objective}>
            {tr("system.goal")} · {tr(`system.goal.${goal.phase}`)}
          </span>
        ) : null}
        <span className="act">{state.label}</span>
        <span className="stat" data-stat="subagents" title={tr("system.stat.subagents")}>
          <Glyph className="ic" name="agent" /> {subCount}
        </span>
        <span className="stat" data-stat="tasks" title={tr("system.stat.tasks")}>
          <Glyph className="ic" name="terminal" /> {taskCount}
        </span>
        <span className="stat" data-stat="tokens" title={usage ? tokenTip(usage) : tr("system.stat.noRecording")}>
          {usage ? compact(tokens) : "—"}
        </span>
        <span className="stat" data-stat="time" title={tr("system.stat.time")}>
          {usage ? millis(time) : "—"}
        </span>
        {row.turn_started_ms ? (
          <span className="el live" data-testid="system-agent-elapsed" title={tr("system.stat.elapsed")}>
            {since(row.turn_started_ms)}
          </span>
        ) : (
          <span className="el" title={row.last_active ?? ""}>{row.last_active ? since(Date.parse(row.last_active)) : ""}</span>
        )}
      </div>
      {expanded ? (
        <>
          {listed?.teams?.length ? (
            <div style={{ paddingLeft: 48 }}>
              <TeamBoard teams={listed.teams} tasks={tasks} />
            </div>
          ) : null}
          <SubAgentRows view={view} instance={row.instance} root={row.session_id} under={subs} parent={row.session_id} depth={1} />
          {own.map((task) => (
            <TaskRow key={task.task_id} view={view} instance={row.instance} sessionId={row.session_id} task={task} depth={1} />
          ))}
          {!subCount && !own.length ? <div className="tr none l1">{tr("system.agents.nothingUnder")}</div> : null}
        </>
      ) : null}
    </div>
  );
}

function SubAgentRows({ view, instance, root, under, parent, depth }: { view: View; instance: string; root: string; under: Map<string, SubAgent[]>; parent: string; depth: number }) {
  const tr = useT();
  const picked = useSelection(view);
  return (
    <>
      {(under.get(parent) ?? []).map((sub) => {
        const status = sub.task?.status ?? "done";
        const label = String(sub.task?.detail?.description ?? sub.task?.detail?.agent_type ?? sub.id);
        const u = sub.usage;
        return (
          <Fragment key={sub.id}>
            <div
              className={`tr sub${isSelected(picked, { kind: "session", instance, sessionId: sub.id }) ? " on" : ""}`}
              style={{ paddingLeft: 28 + depth * 20 }}
              data-testid="system-subagent"
              data-sub={sub.id}
              data-status={status}
              onClick={() => view.select({ kind: "session", instance, sessionId: sub.id, parent: root })}
            >
              <span className={`dot ${status}`} />
              <span className="nm">{tr("system.task.agent")}</span>
              <span className="act">{clip(label, 120)}</span>
              {u?.models[0] ? <span className="kd">{u.models[0]}</span> : null}
              <span className="stat" title={u ? tokenTip([u]) : ""}>{u ? compact(totalTokens(u)) : "—"}</span>
              <span className="stat">{u ? millis(u.model_ms) : sub.task ? taskDuration(sub.task) : "—"}</span>
            </div>
            <SubAgentRows view={view} instance={instance} root={root} under={under} parent={sub.id} depth={depth + 1} />
          </Fragment>
        );
      })}
    </>
  );
}

function TaskRow({ view, instance, sessionId, task, depth }: { view: View; instance: string; sessionId: string; task: Task; depth: number }) {
  const tr = useT();
  const picked = useSelection(view);
  return (
    <div
      className={`tr task${isSelected(picked, { kind: "task", instance, sessionId, taskId: task.task_id }) ? " on" : ""}`}
      style={{ paddingLeft: 28 + depth * 20 }}
      data-task={task.task_id}
      data-status={task.status}
      onClick={() => view.select({ kind: "task", instance, sessionId, taskId: task.task_id })}
    >
      <span className={`dot ${task.status}`} />
      <span className="nm">{tr(`system.task.${task.kind}`)}</span>
      <span className="act">{taskLabel(task)}</span>
      <span className="el">{taskDuration(task)}</span>
    </div>
  );
}
