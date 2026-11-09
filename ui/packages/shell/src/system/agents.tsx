/** The console's Agents: each main Agent on a row, opened into its sub-agents and tasks. */

import { Fragment, memo, useMemo, useState } from "react";
import { clip, useT } from "@harnix/react";

import type { App } from "../app.ts";
import { compact, millis, since } from "../format.ts";
import { Glyph } from "../glyph.tsx";
import { useShell } from "../hooks.ts";
import { rowKeys } from "../roving.ts";
import { LIVE, PAGE, SYSTEM } from "./constants.ts";
import { isActive, stateOf, useAgentRows, useGoals, useUsage } from "./data.ts";
import { goalState, taskDuration, taskLabel, tokenTip, totalTokens } from "./format.ts";
import { TeamBoard } from "./pane/tasks.tsx";
import type { AgentRow, AgentUsage, Goal, InstanceRow, Scope, Task, Team } from "./types.ts";
import { useIsSelected, useTick, useTopic, useTopicIf, type AgentListState, type View } from "./view.ts";

/**
 * The Agents: each main Agent on a row — how it stands, whether Harnix itself started it, and its
 * sub-agents, tasks, tokens and model time — and, opened, its sub-agents as a tree and its tasks.
 * The active tab lists what is doing something or is loaded; the all tab every main Agent of every
 * instance, Harnix's own among them.
 *
 * A grid with a header line, every row of it reachable by Tab and picked with Enter (a treegrid:
 * Right and Left open and close a row, Up and Down go along). Its search, filter and length are the
 * view's, so a tab left and come back to is as it was.
 */
export function Agents({ app, view, scope }: { app: App; view: View; scope: Scope }) {
  const tr = useT();
  const listed = useTopic<{ instances: InstanceRow[] }>(view, "harnix.instances");
  const s = useShell(app);
  const turns = new Set(Object.keys(s.running));
  const rows = useAgentRows(view, listed?.instances ?? []);
  const [kept, keep] = useState<AgentListState>(() => ({ ...view.agentLists[scope] }));
  const change = (next: Partial<AgentListState>) =>
    keep((now) => {
      const made = { ...now, ...next };
      view.agentLists[scope] = made;
      return made;
    });
  const { query, who, limit } = kept;
  const q = query.trim().toLowerCase();
  const matches = (row: AgentRow) =>
    (who === "all" || (row.started_by ?? "user") === who) &&
    (!q || [row.name, row.session_id, row.space, row.instance, row.scene, row.project_root].some((v) => v && String(v).toLowerCase().includes(q)));
  const inScope = scope === "active" ? rows.filter((row) => isActive(row, turns)) : rows;
  const chosen = inScope.filter(matches);
  const rank = (row: AgentRow) => stateOf(row, turns, view.isUnread(row.key)).rank;
  const sorted = [...chosen].sort((a, b) => rank(a) - rank(b) || (b.last_active ?? "").localeCompare(a.last_active ?? ""));
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
          aria-label={tr("system.agents.search")}
          placeholder={tr("system.agents.search")}
          value={query}
          onChange={(e) => change({ query: e.target.value })}
        />
        <select data-testid="system-agents-who" aria-label={tr("system.agents.who")} value={who} onChange={(e) => change({ who: e.target.value as AgentListState["who"] })}>
          <option value="all">{tr("system.agents.who.all")}</option>
          <option value="user">{tr("system.agents.who.user")}</option>
          <option value="harnix">{tr("system.agents.who.harnix")}</option>
        </select>
        <span className="count" data-testid="system-agents-count">{tr("system.agents.count", { count: chosen.length, of: inScope.length })}</span>
      </div>
      {shown.length ? (
        <div className="agents-list" role="treegrid" aria-label={tr(scope === "active" ? "system.agents.active" : "system.agents.all")} data-rows="">
          <div className="tr head" role="row" data-testid="system-agents-head">
            <span className="c-name" role="columnheader">{tr("system.agents.col.agent")}</span>
            <span className="c-state" role="columnheader">{tr("system.agents.col.state")}</span>
            <span className="stat" role="columnheader" data-stat="subagents" title={tr("system.stat.subagents")}>{tr("system.agents.col.subagents")}</span>
            <span className="stat" role="columnheader" data-stat="tasks" title={tr("system.stat.tasks")}>{tr("system.agents.col.tasks")}</span>
            <span className="stat" role="columnheader" data-stat="tokens" title={tr("system.agents.col.tokensTip")}>{tr("system.agents.col.tokens")}</span>
            <span className="stat" role="columnheader" data-stat="time" title={tr("system.stat.time")}>{tr("system.agents.col.time")}</span>
            <span className="el" role="columnheader" title={tr("system.agents.col.lastTip")}>{tr("system.agents.col.last")}</span>
          </div>
          {shown.map((row) => (
            <AgentItem key={row.key} app={app} view={view} row={row} usage={usage[row.key]} goal={goals[row.key] ?? null} turning={turns.has(row.key)} scope={scope} />
          ))}
        </div>
      ) : (
        <div className="empty">{tr(q || who !== "all" ? "system.agents.noMatch" : scope === "active" ? "system.agents.noneActive" : "system.agents.none")}</div>
      )}
      {sorted.length > shown.length ? (
        <button type="button" className="more" data-testid="system-agents-more" onClick={() => change({ limit: limit + PAGE })}>
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

/** A time that counts on by itself, on the shared second: the row around it is not drawn again. */
function Since({ at }: { at: number }) {
  useTick(1000);
  return <>{since(at)}</>;
}

/** How long a task has run, counting while it runs. */
function TaskTime({ task }: { task: Task }) {
  useTick(LIVE.has(task.status) && !task.ended_ms ? 1000 : null);
  return <>{taskDuration(task)}</>;
}

interface AgentItemProps {
  app: App;
  view: View;
  row: AgentRow;
  usage?: AgentUsage[];
  goal: Goal | null;
  /** Whether a turn of it is running, as the shell's own status says. */
  turning: boolean;
  scope: Scope;
}

/** One main Agent's row, and opened, what is under it. Drawn again only when what it shows changed. */
const AgentItem = memo(function AgentItem({ app, view, row, usage, goal, turning, scope }: AgentItemProps) {
  const tr = useT();
  const selected = useIsSelected(view, { kind: "session", instance: row.instance, sessionId: row.session_id });
  // Picked, it is seen: read when it is drawn, which picking it does.
  const state = stateOf(row, new Set(turning ? [row.key] : []), view.isUnread(row.key));
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
  const goalIs = goal ? goalState(goal) : null;
  return (
    <div className={`tgroup${expanded ? " open" : ""}`} role="rowgroup" data-sid={row.session_id} data-instance={row.instance} data-state={state.dot}>
      <div
        className={`tr agent${selected ? " on" : ""}`}
        role="row"
        aria-level={1}
        aria-expanded={expanded}
        aria-selected={selected}
        tabIndex={0}
        data-row=""
        data-testid="system-agent"
        title={[row.name || tr("sidebar.untitled"), state.label, `${row.instance} · ${row.space ?? "sessions"} · ${row.scene ?? ""}`, row.project_root ?? "", row.session_id, row.last_active ?? ""].filter(Boolean).join("\n")}
        onClick={select}
        onKeyDown={rowKeys(select, { open: expanded, set: setOpen })}
      >
        <span className="c-twist" role="gridcell">
          <button
            type="button"
            className="twist"
            tabIndex={-1}
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
        </span>
        <span className={`dot ${state.dot}`} aria-hidden="true" title={state.label} />
        <span className="c-name" role="gridcell">
          <span className="nm">{row.name || tr("sidebar.untitled")}</span>
          {row.started_by === "harnix" ? <span className="kd own" data-testid="system-agent-harnix" title={tr("system.agents.harnixTip")}>Harnix</span> : null}
          <span className="kd">{row.space ?? "sessions"}</span>
          {row.instance !== SYSTEM ? <span className="kd">{row.instance}</span> : null}
          {goal && goalIs && goalIs !== "complete" ? (
            <span className={`kd goal ${goalIs}`} data-testid="system-agent-goal" data-phase={goal.phase} data-goal={goalIs} title={goal.objective}>
              {tr("system.goal")} · {tr(`system.goal.${goalIs}`)}
            </span>
          ) : null}
        </span>
        <span className="act c-state" role="gridcell">{state.label}</span>
        <span className="stat" role="gridcell" data-stat="subagents" title={tr("system.stat.subagents")}>
          {subCount}
        </span>
        <span className="stat" role="gridcell" data-stat="tasks" title={tr("system.stat.tasks")}>
          {taskCount}
        </span>
        <span className="stat" role="gridcell" data-stat="tokens" title={usage ? tokenTip(usage) : tr("system.stat.noRecording")}>
          {usage ? compact(tokens) : "—"}
        </span>
        <span className="stat" role="gridcell" data-stat="time" title={usage ? tr("system.stat.time") : tr("system.stat.noRecording")}>
          {usage ? millis(time) : "—"}
        </span>
        {row.turn_started_ms ? (
          <span className="el live" role="gridcell" data-testid="system-agent-elapsed" title={tr("system.stat.elapsed")}>
            <Since at={row.turn_started_ms} />
          </span>
        ) : (
          <span className="el" role="gridcell" title={row.last_active ?? ""}>
            {row.last_active ? <Since at={Date.parse(row.last_active)} /> : ""}
          </span>
        )}
      </div>
      {expanded ? (
        <>
          {listed?.teams?.length ? (
            <div className="under" role="row" aria-level={2}>
              <div role="gridcell" style={{ paddingLeft: 48 }}>
                <TeamBoard teams={listed.teams} tasks={tasks} />
              </div>
            </div>
          ) : null}
          <SubAgentRows view={view} instance={row.instance} root={row.session_id} under={subs} parent={row.session_id} depth={1} />
          {own.map((task) => (
            <TaskRow key={task.task_id} view={view} instance={row.instance} sessionId={row.session_id} task={task} depth={1} />
          ))}
          {!subCount && !own.length ? (
            <div className="tr none l1" role="row" aria-level={2}>
              <span role="gridcell">{tr("system.agents.nothingUnder")}</span>
            </div>
          ) : null}
        </>
      ) : null}
    </div>
  );
});

function SubAgentRows({ view, instance, root, under, parent, depth }: { view: View; instance: string; root: string; under: Map<string, SubAgent[]>; parent: string; depth: number }) {
  return (
    <>
      {(under.get(parent) ?? []).map((sub) => (
        <Fragment key={sub.id}>
          <SubAgentRow view={view} instance={instance} root={root} sub={sub} depth={depth} />
          <SubAgentRows view={view} instance={instance} root={root} under={under} parent={sub.id} depth={depth + 1} />
        </Fragment>
      ))}
    </>
  );
}

function SubAgentRow({ view, instance, root, sub, depth }: { view: View; instance: string; root: string; sub: SubAgent; depth: number }) {
  const tr = useT();
  const selected = useIsSelected(view, { kind: "session", instance, sessionId: sub.id });
  const status = sub.task?.status ?? "done";
  const label = String(sub.task?.detail?.description ?? sub.task?.detail?.agent_type ?? sub.id);
  const u = sub.usage;
  const select = () => view.select({ kind: "session", instance, sessionId: sub.id, parent: root });
  return (
    <div
      className={`tr sub${selected ? " on" : ""}`}
      role="row"
      aria-level={depth + 1}
      aria-selected={selected}
      tabIndex={0}
      data-row=""
      style={{ paddingLeft: 28 + depth * 20 }}
      data-testid="system-subagent"
      data-sub={sub.id}
      data-status={status}
      onClick={select}
      onKeyDown={rowKeys(select)}
    >
      <span className={`dot ${status}`} aria-hidden="true" />
      <span className="nm" role="gridcell">{tr("system.task.agent")}</span>
      <span className="act" role="gridcell">{clip(label, 120)}</span>
      {u?.models[0] ? <span className="kd" role="gridcell">{u.models[0]}</span> : null}
      <span className="stat" role="gridcell" title={u ? tokenTip([u]) : ""}>{u ? compact(totalTokens(u)) : "—"}</span>
      <span className="stat" role="gridcell">{u ? millis(u.model_ms) : sub.task ? <TaskTime task={sub.task} /> : "—"}</span>
    </div>
  );
}

function TaskRow({ view, instance, sessionId, task, depth }: { view: View; instance: string; sessionId: string; task: Task; depth: number }) {
  const tr = useT();
  const selected = useIsSelected(view, { kind: "task", instance, sessionId, taskId: task.task_id });
  const select = () => view.select({ kind: "task", instance, sessionId, taskId: task.task_id });
  return (
    <div
      className={`tr task${selected ? " on" : ""}`}
      role="row"
      aria-level={depth + 1}
      aria-selected={selected}
      tabIndex={0}
      data-row=""
      style={{ paddingLeft: 28 + depth * 20 }}
      data-task={task.task_id}
      data-status={task.status}
      onClick={select}
      onKeyDown={rowKeys(select)}
    >
      <span className={`dot ${task.status}`} aria-hidden="true" />
      <span className="nm" role="gridcell">{tr(`system.task.${task.kind}`)}</span>
      <span className="act" role="gridcell">{taskLabel(task)}</span>
      <span className="el" role="gridcell">
        <TaskTime task={task} />
      </span>
    </div>
  );
}
