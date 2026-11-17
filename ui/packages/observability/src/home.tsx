/**
 * This package's blocks of the home page (`harnix-app.home.panels`, where the Harnix app defines
 * it): the process at a glance, and the usage of the last 30 days.
 *
 * - **全局统计**: the versions, how long the process has run, its memory and threads, who is
 *   connected (`observability.process`, pushed every few seconds while watched); the instances
 *   (`harnix.instances`); the main Agents of every instance, active and all, and the background
 *   tasks they run (each instance's `harnix.instances.sessions`); the sessions running a turn
 *   (`harnix.sessions.status`); and what waits on a person — the count the home page hands its
 *   blocks, opening the whole list.
 * - **用量**: the usage summary of the system space's 用量 tab, the same component (`summary.tsx`).
 *
 * Every read is a device's own and claims no space: the lists are the whole of them, as a process's
 * figures are.
 */

import { useEffect, useRef, useState, type ReactNode } from "react";
import type { InstanceRow, SessionRow } from "@harnix/core";
import { clock, since, useT, type HostApi } from "@harnix/react";

/** The Harnix app's point for a block of the home page. */
export const HOME_PANELS = "harnix-app.home.panels";

/** What the home page hands its blocks (the Harnix app's `HomeContext`). */
export interface HomeContext {
  waiting: number;
  openWaiting(): void;
}

/** Who is connected, as `observability.process` lists it. */
interface Connection {
  client: string;
  device: string;
  topology: string;
  user_agent?: string;
  since_ms?: number;
}

/** `observability.process`: this process's figures. */
export interface ProcessFigures {
  app?: { name: string; version: string };
  harnix?: string;
  agix?: string | null;
  uptime_secs?: number;
  peak_rss_bytes?: number | null;
  threads?: number | null;
  connections?: Connection[];
}

/** A topic's last value, followed while the component is drawn; undefined until one comes. */
function useTopicValue<T>(host: HostApi, topic: string, params: Record<string, unknown> = {}): T | undefined {
  const key = `${topic}\u0000${JSON.stringify(params)}`;
  const [held, setHeld] = useState<{ key: string; value: T } | null>(null);
  useEffect(() => host.subscribe(topic, params, (value) => setHeld({ key, value: value as T })), [host, key]);
  return held?.key === key ? held.value : undefined;
}

/** Each instance's session list, followed while the component is drawn. */
function useInstanceSessions(host: HostApi, instances: string[]): Record<string, SessionRow[]> {
  const [lists, setLists] = useState<Record<string, SessionRow[]>>({});
  const joined = [...instances].sort().join(",");
  useEffect(() => {
    const offs = instances.map((instance) =>
      host.subscribe("harnix.instances.sessions", { instance }, (value) =>
        setLists((now) => ({ ...now, [instance]: ((value as { sessions?: SessionRow[] } | null)?.sessions ?? []) as SessionRow[] })),
      ),
    );
    return () => offs.forEach((off) => off());
  }, [host, joined]);
  return Object.fromEntries(instances.map((instance) => [instance, lists[instance] ?? []]));
}

/** A main Agent doing something, or loaded and ready to: a turn, a queue, a question, a task, or in memory. */
export function agentActive(row: SessionRow, running: boolean): boolean {
  return Boolean(running || row.running || row.queued || row.waiting || row.tasks || row.status === "active");
}

/** What kind of client a user agent is, roughly. */
function clientKind(agent: string | undefined): "desktop" | "mobile" | "browser" | "script" {
  const ua = agent ?? "";
  if (!ua) return "script";
  if (/Tauri|Electron|harnix-desktop/i.test(ua)) return "desktop";
  if (/Mobile|Android|iPhone|iPad/i.test(ua)) return "mobile";
  if (/Mozilla|WebKit|Chrome|Safari|Firefox/i.test(ua)) return "browser";
  return "script";
}

/** The process, its instances and Agents, and what waits, in one row of figures. */
export function GlobalStats({ host, context }: { host: HostApi; context: HomeContext }) {
  const tr = useT();
  const process = useTopicValue<ProcessFigures>(host, "observability.process");
  // When the figures were last heard: the uptime counts on from there by the second.
  const heard = useRef<{ at: number; value: ProcessFigures | undefined }>({ at: Date.now(), value: undefined });
  if (heard.current.value !== process) heard.current = { at: Date.now(), value: process };
  const instances = useTopicValue<{ instances?: InstanceRow[] }>(host, "harnix.instances")?.instances ?? [];
  const lists = useInstanceSessions(
    host,
    instances.map((i) => i.id),
  );
  const status = useTopicValue<{ sessions?: { instance: string; session_id: string }[] }>(host, "harnix.sessions.status")?.sessions ?? [];
  const turns = new Set(status.map((row) => `${row.instance}/${row.session_id}`));
  const rows = Object.entries(lists).flatMap(([instance, sessions]) => sessions.map((row) => ({ row, running: turns.has(`${instance}/${row.session_id}`) })));
  const active = rows.filter(({ row, running }) => agentActive(row, running)).length;
  const tasks = rows.reduce((n, { row }) => n + (row.tasks ?? 0), 0);
  const users = process?.connections ?? [];
  const cell = (id: string, value: ReactNode, label: string, extra?: ReactNode) => (
    <div key={id} className={`obs-kpi${extra ? " has-tip" : ""}`} data-kpi={id} title={extra ? undefined : typeof value === "string" ? `${label}: ${value}` : label}>
      <div className="v">{value}</div>
      <div className="l">{label}</div>
      {extra}
    </div>
  );
  const version = (v: string | null | undefined) => (v ? v.split("-")[0]! : "—");
  return (
    <div className="obs-kpis" data-testid="home-stats">
      {cell("harnix", version(process?.harnix), "Harnix")}
      {cell("agix", version(process?.agix), "Agix")}
      {cell("uptime", process?.uptime_secs != null ? <Uptime secs={process.uptime_secs} heard={heard.current.at} /> : "—", tr("home.stats.uptime"))}
      {cell("memory", process?.peak_rss_bytes ? `${Math.round(process.peak_rss_bytes / 1048576)} MB` : "—", tr("home.stats.memory"))}
      {cell("threads", process?.threads != null ? String(process.threads) : "—", tr("home.stats.threads"))}
      {cell(
        "users",
        process ? String(users.length) : "—",
        tr("home.stats.users"),
        users.length ? (
          <div className="obs-kpi-tip" data-testid="home-connections">
            {users.map((c) => (
              <div key={c.client} className="row" data-device={c.device}>
                <span className="dev">{c.device}</span>
                <span>{tr(`process.kind.${clientKind(c.user_agent)}`)}</span>
                <span>{c.topology}</span>
                <span className="since">{c.since_ms ? since(c.since_ms) : ""}</span>
              </div>
            ))}
          </div>
        ) : undefined,
      )}
      {cell("instances", String(instances.length), tr("home.stats.instances"))}
      {cell("agents", `${active}/${rows.length}`, tr("home.stats.agents"))}
      {cell("sessions", String(turns.size), tr("home.stats.sessions"))}
      {cell("tasks", String(tasks), tr("home.stats.tasks"))}
      <button type="button" key="waiting" className={`obs-kpi link${context.waiting ? " hot" : ""}`} data-kpi="waiting" data-testid="home-stats-waiting" title={tr("inbox.title")} onClick={() => context.openWaiting()}>
        <div className="v">{String(context.waiting)}</div>
        <div className="l">{tr("home.stats.waiting")}</div>
      </button>
    </div>
  );
}

/** How long the process has run: what it last said, counted on by the second. */
function Uptime({ secs, heard }: { secs: number; heard: number }) {
  const [, bump] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => bump((n) => n + 1), 1000);
    return () => clearInterval(timer);
  }, []);
  return <>{clock(secs + Math.floor((Date.now() - heard) / 1000))}</>;
}
