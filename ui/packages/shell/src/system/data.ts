/** The Agents' figures: their rows, how each stands, what each has used, their goals and tasks. */

import { useEffect, useRef, useState } from "react";
import { t } from "@harnix/react";

import type { App, SessionKey } from "../app.ts";
import { LIVE, RUNNING_USAGE_EVERY, USAGE_BATCH } from "./constants.ts";
import type { AgentRow, AgentUsage, Context, Goal, InstanceRow, SessionRowView, Task, TurnRow } from "./types.ts";
import { useNow, type View } from "./view.ts";

/** Every instance's main Agents, kept current by the instances' session lists. */
export function useAgentRows(view: View, instances: InstanceRow[]): AgentRow[] {
  const [, bump] = useState(0);
  const ids = instances.map((i) => i.id).sort();
  const joined = ids.join(",");
  useEffect(() => {
    const offs = ids.map((instance) => view.watch("harnix.instances.sessions", { instance }, () => bump((n) => n + 1)));
    return () => offs.forEach((off) => off());
  }, [view, joined]);
  return ids.flatMap((instance) => {
    const rows = (view.value("harnix.instances.sessions", { instance }) as { sessions?: SessionRowView[] } | undefined)?.sessions ?? [];
    return rows.map((row) => ({ ...row, instance, key: `${instance}/${row.session_id}` }));
  });
}

/** An Agent doing something, or loaded and ready to: a turn, a queue, a question, a task, or in memory. */
export function isActive(row: AgentRow, turns: Set<string>): boolean {
  return Boolean(turns.has(row.key) || row.running || row.queued || row.waiting || row.tasks || row.status === "active");
}

/** How a main Agent stands, as DSH draws it: what waits on a person first, then what runs. */
export function stateOf(row: AgentRow, turns: Set<string>, unread = false): { dot: string; label: string; rank: number } {
  if (row.waiting === "approval") return { dot: "waiting", label: t("system.state.approval"), rank: 0 };
  if (row.waiting === "answer") return { dot: "waiting", label: t("system.state.answer"), rank: 0 };
  if (row.waiting === "plan") return { dot: "waiting", label: t("system.state.plan"), rank: 0 };
  if (turns.has(row.key) || row.running) return { dot: "running", label: row.queued ? t("system.state.queued", { count: row.queued }) : t("system.state.running"), rank: 1 };
  if (row.tasks) return { dot: "running", label: t("system.state.tasks", { count: row.tasks }), rank: 2 };
  if (unread) return { dot: "done", label: t("system.state.unread"), rank: 3 };
  if (row.status === "active") return { dot: "idle", label: t("system.state.idle"), rank: 3 };
  return { dot: "cold", label: t("system.state.inactive"), rank: 4 };
}

/** What a row says about whether its Agent has done something since: when it changes, its figures
 *  are read again. */
export function signature(row: AgentRow): string {
  return [row.key, row.last_active, row.running, row.turn_started_ms, row.tasks, row.queued, row.waiting].join("|");
}

/**
 * `signature` without `last_active`, for the Agent pane: reading a session's context or turns
 * touches its `last_active`, so a pane that read again on that change would read for ever — and
 * a read in flight can bring a session being deleted back into the engine's list.
 */
export function stateSignature(row: AgentRow): string {
  return [row.key, row.running, row.turn_started_ms, row.tasks, row.queued, row.waiting].join("|");
}

/**
 * What each of these sessions' Agents has used. Read when a row first shows, and again for a row
 * whose pushed state changed (a turn began or ended, a task, a question) — no clock. While a turn
 * runs its calls land in the recording one by one, so a running row is read again as its turn goes.
 */
export function useUsage(app: App, rows: AgentRow[]): Record<string, AgentUsage[]> {
  const [usage, setUsage] = useState<Record<string, AgentUsage[]>>({});
  const seen = useRef(new Map<string, string>());
  const joined = rows.map(signature).join(",");
  const running = rows.some((row) => row.running || row.tasks);
  useNow(running ? RUNNING_USAGE_EVERY : null);
  const tick = running ? Math.floor(Date.now() / RUNNING_USAGE_EVERY) : 0;
  useEffect(() => {
    let alive = true;
    const changed = rows.filter((row) => seen.current.get(row.key) !== signature(row) || row.running || row.tasks);
    for (const row of changed) seen.current.set(row.key, signature(row));
    const by = new Map<string, string[]>();
    for (const row of changed) by.set(row.instance, [...(by.get(row.instance) ?? []), row.session_id]);
    const load = () => {
      for (const [instance, ids] of by) {
        for (let at = 0; at < ids.length; at += USAGE_BATCH) {
          void app
            .call<{ sessions: Record<string, { agents: AgentUsage[] }> }>("harnix.system.usage", { instance, session_ids: ids.slice(at, at + USAGE_BATCH) })
            .then((answer) => {
              if (!alive) return;
              setUsage((now) => {
                const next = { ...now };
                for (const [id, value] of Object.entries(answer.sessions)) next[`${instance}/${id}`] = value.agents;
                return next;
              });
            })
            .catch(() => undefined);
        }
      }
    };
    load();
    return () => {
      alive = false;
    };
  }, [app, joined, tick]);
  return usage;
}


/**
 * The goals of these sessions. The hub keeps what it hears of a goal on the session's row and
 * pushes it with the row; a loaded Agent it has heard nothing of is asked once
 * (`harnix.system.goal`), and the answer, passing through the hub, lands on the row too.
 */
export function useGoals(app: App, rows: AgentRow[]): Record<string, Goal | null> {
  const asked = useRef(new Set<string>());
  const unknown = rows.filter((row) => row.goal === undefined && row.status === "active");
  const joined = unknown.map((row) => row.key).join(",");
  useEffect(() => {
    for (const row of unknown) {
      if (asked.current.has(row.key)) continue;
      asked.current.add(row.key);
      void app.call("harnix.system.goal", { instance: row.instance, session_id: row.session_id }).catch(() => undefined);
    }
  }, [app, joined]);
  return Object.fromEntries(rows.map((row) => [row.key, (row.goal as Goal | null | undefined) ?? null]));
}


/** How many tasks are running across these sessions (`instance/session` keys). */
export function useRunningTasks(view: View, keys: string[]): number {
  const [, bump] = useState(0);
  const joined = [...keys].sort().join(",");
  useEffect(() => {
    const offs = keys.map((key) => {
      const [instance, session_id] = splitKey(key);
      return view.watch("harnix.sessions.tasks", { instance, session_id }, () => bump((n) => n + 1));
    });
    return () => offs.forEach((off) => off());
  }, [view, joined]);
  let count = 0;
  for (const key of keys) {
    const [instance, session_id] = splitKey(key);
    const tasks = (view.value("harnix.sessions.tasks", { instance, session_id }) as { tasks?: Task[] } | undefined)?.tasks ?? [];
    count += tasks.filter((task) => LIVE.has(task.status)).length;
  }
  return count;
}

export function splitKey(key: string): [string, string] {
  const at = key.indexOf("/");
  return [key.slice(0, at), key.slice(at + 1)];
}

/**
 * What the Agent pane reads: its figures this run, its turns, the context of its next call and its
 * goal — again whenever one of its turns settles, and every few seconds while it is busy. A part
 * that cannot be read (a session not in memory has no context to inspect) is left out, not an error.
 */
export function useAgentFacts(app: App, session: SessionKey, busy: boolean, round: unknown = 0) {
  const [facts, setFacts] = useState<{ usage: AgentUsage[] | null; turns: TurnRow[]; context: Context | null; goal: Goal | null }>({ usage: null, turns: [], context: null, goal: null });
  useEffect(() => {
    let alive = true;
    const target = { instance: session.instance, session_id: session.sessionId };
    const quiet = <T,>(p: Promise<T>) => p.catch(() => null);
    const load = async () => {
      const [usage, turns, context, goal] = await Promise.all([
        quiet(app.call<{ sessions: Record<string, { agents: AgentUsage[] }> }>("harnix.system.usage", { instance: session.instance, session_ids: [session.sessionId] })),
        quiet(app.call<{ turns: TurnRow[] }>("harnix.system.turns", target)),
        quiet(app.call<Context>("harnix.system.context", target)),
        quiet(app.call<{ goal: Goal | null }>("harnix.system.goal", target)),
      ]);
      if (!alive) return;
      setFacts({ usage: usage?.sessions[session.sessionId]?.agents ?? null, turns: turns?.turns ?? [], context, goal: goal?.goal ?? null });
    };
    void load();
    const off = app.client.onNotify((method, params) => {
      if (method === "harnix.turn_settled" && params.session_id === session.sessionId) void load();
    });
    // A running turn's calls land one by one with nothing pushed for each: read again while it runs.
    const timer = busy ? setInterval(() => void load(), RUNNING_USAGE_EVERY) : undefined;
    return () => {
      alive = false;
      off();
      clearInterval(timer);
    };
  }, [app, session.instance, session.sessionId, busy, round]);
  return facts;
}
