/**
 * The system space (design §9): what this process runs, and the hands to act on it.
 *
 * Its three columns: on the left, the instances; in the centre, the console — the process at a
 * glance, then the Agents (the active ones, or all of them: each main Agent on a row with its
 * sub-agents, tasks, tokens and model time, opened into its sub-agents and tasks) and the
 * diagnostics; on the right, whatever was picked there:
 * an instance's facts and its restart, or a session's facts and operations, every model call with
 * what went in and what came out, the context the next call will carry, its tasks, and its frames
 * as they arrive. There is no conversation here: the system space looks at sessions, it does not
 * hold one.
 *
 * Registered only when the backend offers the system methods (`[shell] spaces` keeps `system`),
 * and built on the public calls alone: `harnix.system.*` for what the engine keeps behind the
 * system module, `session.*` and `harnix.instances.*` claiming the system space for the rest —
 * every one of them audited. What Agix does not offer has no tab here (§9.3).
 */

import { Fragment, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { errorText } from "@harnix/core";
import { Disclosure, clip, t, useT, type SpaceTab } from "@harnix/react";

import { keyOf, type App, type SessionKey } from "./app.ts";
import { Body, confirmDialog, Connections, exportDialog, Foot, renameDialog, since, type Connection } from "./dialogs.tsx";
import { Glyph } from "./glyph.tsx";
import { useShell } from "./hooks.ts";
import { Diagnostics } from "./settings.tsx";
import { AppsTab, appsOffered } from "./system-apps.tsx";
import { topicKey, topicsOf, type Topics } from "./topics.ts";

export const SYSTEM = "system";
/** Where this browser keeps the sessions that finished unseen, and how many at most. */
const UNREAD_KEY = "harnix.system.unread";
const UNREAD_KEPT = 200;
const OWNER = "builtin";

interface InstanceRow {
  id: string;
  carrier: string;
  template?: string | null;
  params?: Record<string, string>;
  owner: string;
  status: { state: string; reason?: string };
  version?: string | null;
  protocol_version?: number | null;
  data_root?: string;
  scenes?: string[];
  pid?: number | null;
  created_ms?: number | null;
}

interface SessionRowView {
  session_id: string;
  instance?: string;
  name?: string | null;
  scene?: string | null;
  status?: string | null;
  space?: string;
  last_active?: string;
  /** How many of its tasks are running: work that can outlive the turn that started it. */
  tasks?: number;
  running?: boolean;
  queued?: number;
  /** What it waits on a person for: a permission, an answer to the model's question, or a plan's review. */
  waiting?: "approval" | "answer" | "plan" | null;
  /** `harnix`: a module or the kernel started it, not a person. */
  started_by?: "user" | "harnix";
  /** When the running turn began (ms since the epoch). */
  turn_started_ms?: number | null;
  /** Its goal as the hub last heard it; absent when the hub has heard nothing of it. */
  goal?: Goal | null;
  project_root?: string | null;
}

interface Task {
  task_id: string;
  kind: string;
  status: string;
  started_ms?: number | null;
  ended_ms?: number | null;
  detail?: Record<string, unknown>;
  error?: string;
  usage?: { input_tokens?: number; output_tokens?: number; cache_read_input_tokens?: number; cache_creation_input_tokens?: number };
  waiting_for?: string[];
}

/** What the right column is about. `space`: one space's sessions, across instances, the one a
 *  person came from (`current`) first. */
export type Selection =
  | { kind: "space"; space: string; current?: SessionKey }
  | { kind: "instance"; instance: string }
  /** `parent`: a sub-agent's session, under that main Agent's (its calls are in the parent's recording). */
  | { kind: "session"; instance: string; sessionId: string; parent?: string }
  | { kind: "task"; instance: string; sessionId: string; taskId: string };

/** The system space's view, once it is registered: what `openInSystem` drives. */
let registered: View | null = null;

/** Whether this product keeps the system space, so a way into it may be offered. */
export function systemOffered(): boolean {
  return registered !== null;
}

/**
 * Into the system space from a conversation. With `tool`: that session's calls, the call that asked
 * for the tool picked out. Without: the sessions of the space the person came from, theirs first —
 * an application has many, and the one in front is only one of them.
 */
export async function openInSystem(app: App, target: { instance: string; sessionId: string; space: string; tool?: string }): Promise<void> {
  const view = registered;
  if (!view) return;
  await app.switchSpace(SYSTEM);
  const session = { instance: target.instance, sessionId: target.sessionId };
  if (target.tool) {
    view.focusTool(target.tool);
    view.select({ kind: "session", ...session });
    app.selectTab("system.calls", true);
  } else {
    view.select({ kind: "space", space: target.space, current: session });
  }
}

/** Back from the system space to a session where it belongs (the session space lists by time and
 *  by project, so a session of `sessions` may be shown by another), then `then` once it is open. */
function openWhereItBelongs(app: App, session: SessionKey, space: string): void {
  const target = app.registry.at("space").find((c) => (c.sessions ?? c.id) === space && c.id !== SYSTEM)?.id ?? space;
  app.open(session, target);
  void app.switchSpace(target);
}

/**
 * From a call back to the conversation: the session opened where it belongs, and the tool call it
 * asked for (if it asked for one) scrolled to and flashed. The row is drawn once the history is in,
 * so it is looked for a few times.
 */
async function locateInConversation(app: App, session: SessionKey, tool: string | null): Promise<void> {
  const listed = await app.call<{ sessions: SessionRowView[] }>("harnix.sessions", { space: SYSTEM }).catch(() => ({ sessions: [] }));
  const row = listed.sessions.find((r) => r.session_id === session.sessionId && (r.instance ?? "system") === session.instance);
  openWhereItBelongs(app, session, row?.space ?? "sessions");
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

export function registerSystem(app: App): void {
  if (!app.snapshot.reachable.includes("harnix.system.context")) return;
  const view = new View(app);
  registered = view;
  app.registry.register(
    "space",
    {
      id: SYSTEM,
      icon: "cpu",
      order: 90,
      title: () => t("space.system"),
      subtitle: () => t("space.system.subtitle"),
      center: "hidden",
      detailsWidth: 0.45,
      overview: () => <Console app={app} view={view} />,
      sidebar: () => <InstanceList app={app} view={view} />,
      tabs: () => tabsFor(app, view),
    },
    OWNER,
  );
}

/** The right column's tabs for what is selected: only the ones that apply. */
function tabsFor(app: App, view: View): SpaceTab[] {
  const picked = view.selected();
  if (!picked) return [];
  if (picked.kind === "space") {
    return [{ id: "system.overview", title: () => t("system.tab.spaceSessions"), render: () => <SpaceSessions app={app} view={view} space={picked.space} current={picked.current ?? null} /> }];
  }
  if (picked.kind === "instance") {
    return [{ id: "system.overview", title: () => t("system.tab.overview"), render: () => <InstanceOverview app={app} view={view} instance={picked.instance} /> }];
  }
  const session: SessionKey = { instance: picked.instance, sessionId: picked.sessionId };
  // A sub-agent's calls are in its parent's recording and its runtime state in its parent's tasks.
  if (picked.kind === "session" && picked.parent) {
    const parent = picked.parent;
    return [{ id: "system.overview", title: () => t("system.tab.overview"), render: () => <SubAgentOverview app={app} view={view} session={session} parent={parent} /> }];
  }
  const tabs: SpaceTab[] = [
    { id: "system.overview", title: () => t("system.tab.overview"), render: () => <SessionOverview app={app} view={view} session={session} /> },
    { id: "system.calls", title: () => t("system.tab.calls"), render: () => <CallsTab app={app} view={view} session={session} /> },
    { id: "system.turns", title: () => t("system.tab.turns"), render: () => <TurnsTab app={app} session={session} /> },
    { id: "system.context", title: () => t("system.tab.context"), render: () => <ContextTab app={app} view={view} session={session} /> },
    { id: "system.tasks", title: () => t("system.tab.tasks"), render: () => <TasksTab app={app} view={view} session={session} /> },
    { id: "system.frames", title: () => t("system.tab.frames"), render: () => <FramesTab app={app} session={session} /> },
  ];
  // A task opens on its own tab.
  if (picked.kind === "task") tabs.unshift(tabs.splice(tabs.findIndex((x) => x.id === "system.tasks"), 1)[0]!);
  return tabs;
}

/* ── what is selected, and the topics that keep the view current ──────── */

/**
 * The selection, and the hub topics this space watches. A topic is subscribed once however many
 * parts of the page read it, and given up when the last one stops.
 */
class View {
  private picked: Selection | null = null;
  /** A tool call to pick out in the calls tab, once: what a jump from the conversation asked for. */
  private tool: string | null = null;
  private readonly listeners = new Set<() => void>();
  /** The page's one table of subscriptions, shared with the shell: one per connection and key. */
  private readonly table: Topics;
  /** The last value of each topic watched here, while something here watches it. */
  private readonly values = new Map<string, { value: unknown; readers: number }>();
  /** Sessions whose turn ended while they were neither open nor picked here (`instance/session`):
   *  DSH's "completed", until someone looks. */
  private readonly unread = new Set<string>();
  /** The sessions each instance's list last showed running. */
  private readonly wasRunning = new Map<string, Set<string>>();

  constructor(private readonly app: App) {
    this.table = topicsOf(app.client);
    try {
      for (const key of JSON.parse(localStorage.getItem(UNREAD_KEY) ?? "[]") as string[]) this.unread.add(key);
    } catch {
      // No storage, or nothing kept: nothing is unread yet.
    }
  }

  private keepUnread(): void {
    try {
      localStorage.setItem(UNREAD_KEY, JSON.stringify([...this.unread].slice(-UNREAD_KEPT)));
    } catch {
      // Without storage the marks last as long as the page.
    }
  }

  isUnread(key: string): boolean {
    return this.unread.has(key);
  }

  /** A session list arrived: a turn that was running there and is not now, and that nobody here is
   *  looking at, finished unseen. */
  private noteRunning(instance: string, value: unknown): void {
    const rows = (value as { sessions?: SessionRowView[] } | undefined)?.sessions ?? [];
    const now = new Set(rows.filter((r) => r.running).map((r) => r.session_id));
    const before = this.wasRunning.get(instance);
    this.wasRunning.set(instance, now);
    if (!before) return;
    const open = this.app.snapshot.current;
    const picked = this.picked && this.picked.kind === "session" ? `${this.picked.instance}/${this.picked.sessionId}` : null;
    for (const id of before) {
      const key = `${instance}/${id}`;
      if (now.has(id) || key === picked || (open && open.instance === instance && open.sessionId === id)) continue;
      this.unread.add(key);
      this.keepUnread();
    }
  }

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  readonly selected = (): Selection | null => this.picked;

  focusTool(id: string): void {
    this.tool = id;
  }

  /** The tool call to pick out, given up once read. */
  takeTool(): string | null {
    const id = this.tool;
    this.tool = null;
    return id;
  }

  select(selection: Selection): void {
    this.picked = selection;
    if ((selection.kind === "session" || selection.kind === "task") && this.unread.delete(`${selection.instance}/${selection.sessionId}`)) this.keepUnread();
    for (const fn of [...this.listeners]) fn();
    // The right column is where a selection is shown; redraw it with the new tabs.
    this.app.set({ detail: null });
    this.app.toggleDetails(true);
  }

  watch(topic: string, params: Record<string, unknown> | undefined, fn: () => void): () => void {
    const key = topicKey(topic, params);
    const entry = this.values.get(key) ?? { value: undefined, readers: 0 };
    this.values.set(key, entry);
    entry.readers += 1;
    const off = this.table.subscribe(topic, params ?? {}, (value) => {
      entry.value = value;
      if (topic === "harnix.instances.sessions" && typeof params?.instance === "string") this.noteRunning(params.instance, value);
      fn();
    });
    return () => {
      off();
      entry.readers -= 1;
      if (entry.readers === 0 && this.values.get(key) === entry) this.values.delete(key);
    };
  }

  value(topic: string, params?: Record<string, unknown>): unknown {
    return this.values.get(topicKey(topic, params))?.value;
  }
}

function useSelection(view: View): Selection | null {
  return useSyncExternalStore(view.subscribe, view.selected);
}

function useTopic<T>(view: View, topic: string, params?: Record<string, unknown>): T | undefined {
  const key = topicKey(topic, params);
  const [, bump] = useState(0);
  useEffect(() => view.watch(topic, params, () => bump((n) => n + 1)), [view, key]);
  return view.value(topic, params) as T | undefined;
}

/** Draw again every `every` ms while given: for clocks that count on their own. */
function useNow(every: number | null): void {
  const [, bump] = useState(0);
  useEffect(() => {
    if (!every) return;
    const timer = setInterval(() => bump((n) => n + 1), every);
    return () => clearInterval(timer);
  }, [every]);
}

/** `useTopic`, watching only while `on`. */
function useTopicIf<T>(view: View, topic: string, params: Record<string, unknown>, on: boolean): T | undefined {
  const key = topicKey(topic, params);
  const [, bump] = useState(0);
  useEffect(() => (on ? view.watch(topic, params, () => bump((n) => n + 1)) : undefined), [view, key, on]);
  return on ? (view.value(topic, params) as T | undefined) : undefined;
}

/** A call made from this space, with its failure said in the banner. */
async function act(app: App, run: () => Promise<unknown>): Promise<boolean> {
  try {
    await run();
    return true;
  } catch (e) {
    app.notice(errorText(e), "error");
    return false;
  }
}

function isSelected(picked: Selection | null, row: Selection): boolean {
  if (!picked || picked.kind === "space" || picked.kind !== row.kind || picked.instance !== row.instance) return false;
  if (row.kind === "instance") return true;
  if (picked.kind === "instance" || picked.sessionId !== (row as { sessionId: string }).sessionId) return false;
  return row.kind === "session" || (picked as { taskId: string }).taskId === (row as { taskId: string }).taskId;
}

/* ── the left column: the instances ───────────────────────────────────── */

function InstanceList({ app, view }: { app: App; view: View }) {
  const tr = useT();
  const picked = useSelection(view);
  const listed = useTopic<{ instances: InstanceRow[] }>(view, "harnix.instances");
  const s = useShell(app);
  return (
    <div className="system-tree" data-testid="system-tree">
      <div className="sec">
        <span className="truncate">{tr("system.instances")}</span>
      </div>
      {(listed?.instances ?? []).map((instance) => {
        const running = Object.keys(s.running).filter((k) => k.startsWith(`${instance.id}/`)).length;
        return (
          <button
            type="button"
            key={instance.id}
            className={`srow inst${isSelected(picked, { kind: "instance", instance: instance.id }) ? " on" : ""}`}
            data-instance={instance.id}
            data-state={instance.status.state}
            title={instance.status.reason ?? instance.status.state}
            onClick={() => view.select({ kind: "instance", instance: instance.id })}
          >
            <span className={`dot ${instance.status.state}`} />
            <span className="tt name">{instance.id}</span>
            <span className="meta carrier">{running || ""}</span>
          </button>
        );
      })}
    </div>
  );
}

/* ── the centre: the console ──────────────────────────────────────────── */

interface Process {
  app?: { name: string; version: string };
  harnix?: string;
  agix?: string | null;
  uptime_secs?: number;
  peak_rss_bytes?: number | null;
  threads?: number | null;
  connections?: Connection[];
}

type Scope = "active" | "all";

function Console({ app, view }: { app: App; view: View }) {
  const tr = useT();
  const [tab, setTab] = useState<Scope | "apps" | "diagnostics">("active");
  const tabs: [Scope | "apps" | "diagnostics", string][] = [
    ["active", tr("system.agents.active")],
    ["all", tr("system.agents.all")],
    // The runtime applications, where this process hosts them.
    ...(appsOffered(app) ? [["apps", tr("system.apps")] as ["apps", string]] : []),
    ["diagnostics", tr("system.diagnostics")],
  ];
  return (
    <div className="system-console" data-testid="system-console">
      <Strip app={app} view={view} />
      <div className="stabs" role="tablist">
        {tabs.map(([id, label]) => (
          <button key={id} type="button" role="tab" data-tab={id} aria-selected={tab === id} className={tab === id ? "on" : ""} onClick={() => setTab(id)}>
            {label}
          </button>
        ))}
      </div>
      {tab === "diagnostics" ? (
        <div className="system-diagnostics settings-page"><Diagnostics app={app} /></div>
      ) : tab === "apps" ? (
        <AppsTab app={app} />
      ) : (
        <Agents key={tab} app={app} view={view} scope={tab} />
      )}
    </div>
  );
}

/** The process in one row: versions, how long it has run, its memory and threads, who is connected, and the Agents. */
function Strip({ app, view }: { app: App; view: View }) {
  const tr = useT();
  const s = useShell(app);
  // Pushed (the hub computes it again every few seconds while it is watched); the uptime goes on by
  // the second here between two pushes.
  const process = useTopic<Process>(view, "harnix.process");
  const heard = useRef<{ at: number; value: Process | undefined }>({ at: Date.now(), value: undefined });
  if (heard.current.value !== process) heard.current = { at: Date.now(), value: process };
  useNow(1000);
  const instances = useTopic<{ instances: InstanceRow[] }>(view, "harnix.instances");
  const rows = useAgentRows(view, instances?.instances ?? []);
  const turns = new Set(Object.keys(s.running));
  const busy = rows.filter((row) => row.tasks).map((row) => row.key);
  const tasks = useRunningTasks(view, [...new Set([...turns, ...busy])]);
  const users = process?.connections ?? [];
  const kpi = (id: string, value: string, label: string, extra?: React.ReactNode) => (
    <div className={`kpi${extra ? " has-tip" : ""}`} data-kpi={id} title={value}>
      <div className="v">{id === "harnix" || id === "agix" ? value.split("-")[0] : value}</div>
      <div className="l">{label}</div>
      {extra}
    </div>
  );
  return (
    <div className="strip" data-testid="system-strip">
      {kpi("harnix", process?.harnix ?? "—", "Harnix")}
      {kpi("agix", process?.agix ?? "—", "Agix")}
      {kpi("uptime", process?.uptime_secs != null ? uptime(process.uptime_secs + Math.floor((Date.now() - heard.current.at) / 1000)) : "—", tr("system.uptime"))}
      {kpi("memory", process?.peak_rss_bytes ? `${Math.round(process.peak_rss_bytes / 1048576)} MB` : "—", tr("system.peakMemory"))}
      {kpi("threads", process?.threads != null ? String(process.threads) : "—", tr("system.threads"))}
      {kpi(
        "users",
        String(users.length),
        tr("system.users"),
        <div className="tip2">
          <div className="tip2-h">{tr("process.users")}</div>
          <Connections connections={users} />
        </div>,
      )}
      {kpi("instances", String(instances?.instances.length ?? 0), tr("system.instances"))}
      {kpi("agents", `${rows.filter((row) => isActive(row, turns)).length}/${rows.length}`, tr("system.kpi.agents"))}
      {kpi("sessions", String(turns.size), tr("system.runningSessions"))}
      {kpi("tasks", String(tasks), tr("system.runningTasks"))}
      {kpi("waiting", String(rows.filter((row) => row.waiting).length), tr("system.kpi.waiting"))}
    </div>
  );
}

function uptime(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return h ? `${h}:${String(m).padStart(2, "0")}:${String(secs % 60).padStart(2, "0")}` : `${m}:${String(secs % 60).padStart(2, "0")}`;
}

const LIVE = new Set(["running", "waiting", "pending"]);

/** A main Agent: a top-level session of an instance, as the instance's session list has it. */
interface AgentRow extends SessionRowView {
  instance: string;
  /** `instance/session`. */
  key: string;
}

/** What one Agent of a session has used in its current run (`harnix.system.usage`). */
interface AgentUsage {
  session_id: string;
  parent_session_id?: string | null;
  calls: number;
  turns: number;
  models: string[];
  input_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  output_tokens: number;
  model_ms: number;
  errors: number;
  last_at?: number | null;
}

/** Every instance's main Agents, kept current by the instances' session lists. */
function useAgentRows(view: View, instances: InstanceRow[]): AgentRow[] {
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
function isActive(row: AgentRow, turns: Set<string>): boolean {
  return Boolean(turns.has(row.key) || row.running || row.queued || row.waiting || row.tasks || row.status === "active");
}

/** How a main Agent stands, as DSH draws it: what waits on a person first, then what runs. */
function stateOf(row: AgentRow, turns: Set<string>, unread = false): { dot: string; label: string; rank: number } {
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
function signature(row: AgentRow): string {
  return [row.key, row.last_active, row.running, row.turn_started_ms, row.tasks, row.queued, row.waiting].join("|");
}

/**
 * What each of these sessions' Agents has used. Read when a row first shows, and again for a row
 * whose pushed state changed (a turn began or ended, a task, a question) — no clock. While a turn
 * runs its calls land in the recording one by one, so a running row is read again as its turn goes.
 */
function useUsage(app: App, rows: AgentRow[]): Record<string, AgentUsage[]> {
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

/** While a turn runs nothing is pushed per model call: how often its row's figures are read then. */
const RUNNING_USAGE_EVERY = 3000;

/**
 * The goals of these sessions. The hub keeps what it hears of a goal on the session's row and
 * pushes it with the row; a loaded Agent it has heard nothing of is asked once
 * (`harnix.system.goal`), and the answer, passing through the hub, lands on the row too.
 */
function useGoals(app: App, rows: AgentRow[]): Record<string, Goal | null> {
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

/** One `harnix.system.usage` call's most. */
const USAGE_BATCH = 200;
/** A sub-agent's transcript, read a page of this many messages at a time. */
const SUB_PAGE = 30;
/** How many main Agents the all tab lists before "more". */
const PAGE = 100;

/** How many tasks are running across these sessions (`instance/session` keys). */
function useRunningTasks(view: View, keys: string[]): number {
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

function splitKey(key: string): [string, string] {
  const at = key.indexOf("/");
  return [key.slice(0, at), key.slice(at + 1)];
}

/**
 * The Agents: each main Agent on a row — how it stands, whether Harnix itself started it, and its
 * sub-agents, tasks, tokens and model time — and, opened, its sub-agents as a tree and its tasks.
 * The active tab lists what is doing something or is loaded; the all tab every main Agent of every
 * instance, Harnix's own among them.
 */
function Agents({ app, view, scope }: { app: App; view: View; scope: Scope }) {
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

function totalTokens(u: AgentUsage): number {
  return u.input_tokens + u.cache_read_tokens + u.cache_write_tokens + u.output_tokens;
}

function tokenTip(u: AgentUsage[]): string {
  const sum = (k: keyof AgentUsage) => u.reduce((n, a) => n + Number(a[k] ?? 0), 0);
  return t("system.tokenTip", { input: sum("input_tokens"), read: sum("cache_read_tokens"), write: sum("cache_write_tokens"), output: sum("output_tokens") });
}

function ms(n: number): string {
  if (n < 1000) return `${n}ms`;
  const secs = n / 1000;
  return secs < 60 ? `${secs.toFixed(1)}s` : `${Math.floor(secs / 60)}m${String(Math.round(secs % 60)).padStart(2, "0")}s`;
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
          {usage ? ms(time) : "—"}
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
              <span className="stat">{u ? ms(u.model_ms) : sub.task ? duration(sub.task) : "—"}</span>
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
      <span className="el">{duration(task)}</span>
    </div>
  );
}

/* ── right column: an instance ────────────────────────────────────────── */

function Facts({ rows }: { rows: [string, string][] }) {
  return (
    <dl className="facts">
      {rows
        .filter(([, value]) => value)
        .map(([label, value]) => (
          <div key={label} className="fact">
            <dt>{label}</dt>
            <dd title={value}>{value}</dd>
          </div>
        ))}
    </dl>
  );
}

interface Template {
  id: string;
  carrier: string;
  params: string[];
}

function InstanceOverview({ app, view, instance: id }: { app: App; view: View; instance: string }) {
  const tr = useT();
  const listed = useTopic<{ instances: InstanceRow[] }>(view, "harnix.instances");
  const instance = listed?.instances.find((i) => i.id === id);
  return (
    <div className="system-pane" data-testid="system-instance">
      {instance ? (
        <>
          <Facts
            rows={[
              [tr("system.fact.instance"), instance.id],
              [tr("system.fact.state"), instance.status.reason ? `${instance.status.state} — ${instance.status.reason}` : instance.status.state],
              [tr("system.fact.carrier"), tr(`system.carrier.${instance.carrier}`)],
              [tr("system.fact.template"), instance.template ?? ""],
              [tr("system.fact.owner"), instance.owner],
              [tr("system.fact.version"), instance.version ? `${instance.version} · v${instance.protocol_version ?? "?"}` : ""],
              [tr("system.fact.scenes"), (instance.scenes ?? []).join(", ")],
              [tr("system.fact.pid"), instance.pid != null ? String(instance.pid) : ""],
              [tr("system.fact.dataRoot"), instance.data_root ?? ""],
            ]}
          />
          {instance.id !== "system" ? (
            <div className="ops">
              <button
                type="button"
                className="btn sm outline op"
                data-testid="inst-restart"
                onClick={() => void act(app, () => app.call("harnix.instances.restart", { instance: instance.id, space: SYSTEM }))}
              >
                <Glyph name="refresh" className="glyph" />
                {tr("system.restart")}
              </button>
              {instance.owner !== "kernel" ? (
                <button
                  type="button"
                  className="btn sm outline op danger"
                  data-testid="inst-stop"
                  onClick={() =>
                    confirmDialog(app, tr("system.confirmStop", { instance: instance.id }), async () => {
                      await app.call("harnix.instances.stop", { instance: instance.id, space: SYSTEM });
                      app.closeModal();
                      view.select({ kind: "instance", instance: "system" });
                    })
                  }
                >
                  <Glyph name="stop" className="glyph" />
                  {tr("system.stop")}
                </button>
              ) : null}
            </div>
          ) : null}
          <div className="label">{tr("system.sessionsOf")}</div>
          <InstanceSessions view={view} instance={instance.id} />
        </>
      ) : (
        <div className="empty">{tr("system.pickInstance")}</div>
      )}
      <div className="label">{tr("system.newInstance")}</div>
      <NewInstance app={app} view={view} />
    </div>
  );
}

/** Every session of an instance, running or not, of whatever space: one click to inspect it. */
function InstanceSessions({ view, instance }: { view: View; instance: string }) {
  const tr = useT();
  const params = useMemo(() => ({ instance }), [instance]);
  const listed = useTopic<{ sessions: SessionRowView[] }>(view, "harnix.instances.sessions", params);
  const rows = [...(listed?.sessions ?? [])].sort((a, b) => String(b.last_active ?? "").localeCompare(String(a.last_active ?? "")));
  if (!rows.length) return <div className="note">{tr("sidebar.noSessions")}</div>;
  return (
    <div className="inst-sessions" data-instance={instance}>
      {rows.map((row) => (
        <button
          type="button"
          key={row.session_id}
          className="srow sub"
          data-sid={row.session_id}
          data-instance={instance}
          title={[row.name, row.scene, row.project_root].filter(Boolean).join(" · ")}
          onClick={() => view.select({ kind: "session", instance, sessionId: row.session_id })}
        >
          <span className="tt name">{row.name || tr("sidebar.untitled")}</span>
          <span className="space-tag" data-space={row.space}>
            {row.space ?? "sessions"}
          </span>
        </button>
      ))}
    </div>
  );
}

function NewInstance({ app, view }: { app: App; view: View }) {
  const tr = useT();
  const [templates, setTemplates] = useState<Template[] | null>(null);
  const [template, setTemplate] = useState("");
  const [name, setName] = useState("");
  const [values, setValues] = useState<Record<string, string>>({});
  useEffect(() => {
    void app
      .call<{ templates: Template[] }>("harnix.instances.templates", { space: SYSTEM })
      .then((r) => {
        setTemplates(r.templates ?? []);
        setTemplate((r.templates ?? [])[0]?.id ?? "");
      })
      .catch(() => setTemplates([]));
  }, [app]);
  if (!templates) return null;
  if (!templates.length) return <div className="note">{tr("system.noTemplates")}</div>;
  const chosen = templates.find((x) => x.id === template);
  const create = () =>
    void act(app, async () => {
      const built = await app.call<{ id?: string; instance?: { id: string } }>("harnix.instances.create", {
        template,
        name: name.trim(),
        params: Object.fromEntries((chosen?.params ?? []).map((p) => [p, values[p] ?? ""])),
        space: SYSTEM,
      });
      setName("");
      view.select({ kind: "instance", instance: built.instance?.id ?? built.id ?? name.trim() });
    });
  return (
    <div className="new-instance" data-testid="new-instance">
      <div className="field">
        <label>{tr("system.fact.template")}</label>
        <select value={template} onChange={(e) => setTemplate(e.target.value)}>
          {templates.map((x) => (
            <option key={x.id} value={x.id}>
              {x.id} · {tr(`system.carrier.${x.carrier}`)}
            </option>
          ))}
        </select>
      </div>
      <div className="field">
        <label>{tr("common.name")}</label>
        <input type="text" data-testid="new-instance-name" value={name} onChange={(e) => setName(e.target.value)} />
      </div>
      {(chosen?.params ?? []).map((param) => (
        <div className="field" key={param}>
          <label>{param}</label>
          <input type="text" value={values[param] ?? ""} onChange={(e) => setValues({ ...values, [param]: e.target.value })} />
        </div>
      ))}
      <button type="button" className="btn sm primary" data-testid="new-instance-create" disabled={!name.trim() || !template} onClick={create}>
        {tr("system.create")}
      </button>
    </div>
  );
}

/* ── right column: a session ──────────────────────────────────────────── */

/** One space's sessions, every instance's: the one a person came from first, then by when last active. */
function SpaceSessions({ app, view, space, current }: { app: App; view: View; space: string; current: SessionKey | null }) {
  const tr = useT();
  const [rows, setRows] = useState<SessionRowView[] | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    const load = () =>
      app
        .call<{ sessions: SessionRowView[] }>("harnix.sessions", { space: SYSTEM })
        .then((r) => alive && setRows((r.sessions ?? []).filter((row) => (row.space ?? "sessions") === space)))
        .catch((e) => alive && setFailure(errorText(e)));
    void load();
    const off = app.client.onNotify((method) => {
      if (method === "harnix.turn_settled") void load();
    });
    return () => {
      alive = false;
      off();
    };
  }, [app, space]);
  if (failure) return <div className="empty">{failure}</div>;
  if (!rows) return null;
  const isCurrent = (row: SessionRowView) => current != null && row.session_id === current.sessionId && (row.instance ?? "system") === current.instance;
  const sorted = [...rows].sort((a, b) => Number(isCurrent(b)) - Number(isCurrent(a)) || String(b.last_active ?? "").localeCompare(String(a.last_active ?? "")));
  return (
    <div className="system-pane" data-testid="system-space" data-space={space}>
      <div className="label">{tr("system.spaceSessionsCount", { space, count: rows.length })}</div>
      {!sorted.length ? <div className="note">{tr("sidebar.noSessions")}</div> : null}
      <div className="inst-sessions">
        {sorted.map((row) => {
          const instance = row.instance ?? "system";
          return (
            <button
              type="button"
              key={`${instance}/${row.session_id}`}
              className={`srow sub${isCurrent(row) ? " on" : ""}`}
              data-sid={row.session_id}
              data-instance={instance}
              data-current={isCurrent(row) ? "true" : undefined}
              title={[row.name, row.scene, row.project_root].filter(Boolean).join(" · ")}
              onClick={() => view.select({ kind: "session", instance, sessionId: row.session_id })}
            >
              <span className="tt name">{row.name || tr("sidebar.untitled")}</span>
              {isCurrent(row) ? <span className="space-tag">{tr("system.currentSession")}</span> : null}
              <span className="space-tag">{instance}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

/** A session's goal (`session.goal`): one aim the engine keeps opening turns for. */
interface Goal {
  objective: string;
  phase: "active" | "paused" | "blocked" | "complete";
  armed: boolean;
  max_rounds: number;
  rounds_started: number;
  blocked_reason?: { code?: string; message?: string } | string | null;
}

/** One `harnix.system.turns` row: a turn's calls summed. */
interface TurnRow {
  turn: number;
  calls: number[];
  ttft_ms?: number | null;
  model_ms: number;
  tool_ms: number;
  input_tokens: number;
  output_tokens: number;
  cache_hit?: number | null;
  tokens_per_s?: number | null;
}

/**
 * What the Agent pane reads: its figures this run, its turns, the context of its next call and its
 * goal — again whenever one of its turns settles, and every few seconds while it is busy. A part
 * that cannot be read (a session not in memory has no context to inspect) is left out, not an error.
 */
function useAgentFacts(app: App, session: SessionKey, busy: boolean, round: unknown = 0) {
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

/** The figures DSH puts by the composer, for one Agent: its turns and calls, its tokens, how fast. */
function Stats({ own, turns }: { own: AgentUsage | undefined; turns: TurnRow[] }) {
  const tr = useT();
  if (!own && !turns.length) return <div className="note" data-testid="agent-stats-none">{tr("system.stat.noRecording")}</div>;
  const sum = (f: (t: TurnRow) => number) => turns.reduce((n, t) => n + f(t), 0);
  const ttfts = turns.map((t) => t.ttft_ms).filter((v): v is number => typeof v === "number");
  const output = own?.output_tokens ?? sum((t) => t.output_tokens);
  // Tokens a second, weighted by what each turn wrote.
  const rated = turns.filter((t) => t.tokens_per_s && t.output_tokens);
  const writing = rated.reduce((n, t) => n + t.output_tokens / t.tokens_per_s!, 0);
  const rate = writing > 0 ? rated.reduce((n, t) => n + t.output_tokens, 0) / writing : null;
  const input = own ? own.input_tokens + own.cache_read_tokens + own.cache_write_tokens : sum((t) => t.input_tokens);
  const hit = own && input > 0 ? own.cache_read_tokens / input : null;
  const cells: [string, string, string][] = [
    ["turns", tr("system.stat.turns"), String(own?.turns ?? turns.length)],
    ["calls", tr("system.stat.calls"), String(own?.calls ?? sum((t) => t.calls.length))],
    ["total", tr("system.stat.total"), compact(input + output)],
    ["hit", tr("system.stat.cacheHit"), hit === null ? "—" : `${Math.round(hit * 100)}%`],
    ["rate", tr("system.stat.rate"), rate === null ? "—" : `${rate.toFixed(1)}/s`],
    ["ttft", tr("system.stat.ttft"), ttfts.length ? ms(Math.round(ttfts.reduce((a, b) => a + b, 0) / ttfts.length)) : "—"],
    ["model", tr("system.stat.time"), ms(own?.model_ms ?? sum((t) => t.model_ms))],
    ["tool", tr("system.stat.toolTime"), turns.length ? ms(sum((t) => t.tool_ms)) : "—"],
  ];
  return (
    <>
      <div className="stat-grid" data-testid="agent-stats">
        {cells.map(([id, label, value]) => (
          <div key={id} className="cell" data-cell={id}>
            <div className="v">{value}</div>
            <div className="l">{label}</div>
          </div>
        ))}
      </div>
      {own ? (
        <div className="note" data-testid="agent-tokens">
          {tr("system.tokenTip", { input: own.input_tokens, read: own.cache_read_tokens, write: own.cache_write_tokens, output: own.output_tokens })}
          {own.errors ? ` · ${tr("system.stat.errors", { count: own.errors })}` : ""}
        </div>
      ) : null}
    </>
  );
}

/** How full the next call's context is, against the window: the provider's count when there is one. */
function ContextRing({ context }: { context: Context | null }) {
  const tr = useT();
  if (!context) return null;
  const used = context.measured?.tokens ?? context.total_tokens ?? 0;
  const window = context.measured?.window ?? 0;
  const pct = window ? Math.min(100, Math.round((used / window) * 100)) : null;
  return (
    <div className="ctx-ring" data-testid="agent-context" data-pct={pct ?? undefined}>
      <div className="ring" style={{ "--pct": `${pct ?? 0}%` } as React.CSSProperties}>
        <span>{pct === null ? "—" : `${pct}%`}</span>
      </div>
      <div className="txt">
        <div>{window ? tr("system.tokensOf", { count: used, of: window }) : tr("system.tokens", { count: `~${used}` })}</div>
        <div className="note">{context.measured ? tr("system.fact.measured") : tr("system.fact.estimated")}</div>
      </div>
    </div>
  );
}

/**
 * A session's goal, and what a person may do with it from here (Agix `session.setGoal` /
 * `updateGoal`, as the system module): set one when there is none — the engine starts its first
 * round at once on an idle session — and pause, resume, edit or clear it.
 */
function GoalBlock({ app, session, goal, changed }: { app: App; session: SessionKey; goal: Goal | null; changed: () => void }) {
  const tr = useT();
  const target = { instance: session.instance, session_id: session.sessionId };
  const update = (action: string, extra: Record<string, unknown> = {}) =>
    void act(app, () => app.call("harnix.system.goal.update", { ...target, action, ...extra })).then(changed);
  const edit = (current: Goal | null) =>
    app.openModal({
      title: tr(current ? "system.goal.edit" : "system.goal.set"),
      content: (
        <GoalForm
          goal={current}
          save={async (objective, rounds) => {
            await (current
              ? app.call("harnix.system.goal.update", { ...target, action: "edit", objective, max_rounds: rounds })
              : app.call("harnix.system.goal.set", { ...target, objective, max_rounds: rounds }));
            app.closeModal();
            changed();
          }}
          cancel={() => app.closeModal()}
        />
      ),
    });
  if (!goal) {
    return (
      <button type="button" className="btn sm outline wide" data-testid="agent-goal-set" onClick={() => edit(null)}>
        {tr("system.goal.set")}
      </button>
    );
  }
  const reason = typeof goal.blocked_reason === "string" ? goal.blocked_reason : goal.blocked_reason?.message;
  const running = goal.phase === "active" && goal.armed;
  return (
    <div className="goal" data-testid="agent-goal" data-phase={goal.phase} data-armed={goal.armed ? "" : undefined}>
      <div className="label">
        {tr("system.goal")}
        <span className={`kd phase ${goal.phase}`}>{tr(`system.goal.${goal.phase}`)}</span>
        {goal.phase === "active" && !goal.armed ? <span className="kd">{tr("system.goal.disarmed")}</span> : null}
      </div>
      <div className="objective">{goal.objective}</div>
      <div className="note">
        {tr("system.goal.rounds", { count: goal.rounds_started, of: goal.max_rounds })}
        {reason ? ` · ${reason}` : ""}
      </div>
      {goal.phase === "complete" ? null : (
        <div className="ops">
          {running ? (
            <button type="button" className="btn sm outline" data-testid="agent-goal-pause" onClick={() => update("pause")}>
              {tr("system.goal.pause")}
            </button>
          ) : (
            <button type="button" className="btn sm outline" data-testid="agent-goal-resume" onClick={() => update("resume")}>
              {tr("system.goal.resume")}
            </button>
          )}
          <button type="button" className="btn sm outline" data-testid="agent-goal-edit" onClick={() => edit(goal)}>
            {tr("system.goal.edit")}
          </button>
          <button
            type="button"
            className="btn sm outline danger"
            data-testid="agent-goal-clear"
            onClick={() =>
              confirmDialog(app, tr("system.goal.confirmClear"), async () => {
                await app.call("harnix.system.goal.update", { ...target, action: "clear" });
                app.closeModal();
                changed();
              })
            }
          >
            {tr("system.goal.clear")}
          </button>
        </div>
      )}
    </div>
  );
}

function GoalForm({ goal, save, cancel }: { goal: Goal | null; save: (objective: string, rounds: number) => Promise<void>; cancel: () => void }) {
  const tr = useT();
  const [objective, setObjective] = useState(goal?.objective ?? "");
  const [rounds, setRounds] = useState(String(goal?.max_rounds ?? 256));
  const [error, setError] = useState<string | null>(null);
  const go = async () => {
    const n = Number(rounds);
    if (!objective.trim() || !Number.isInteger(n) || n < 1) return setError(tr("system.goal.invalid"));
    try {
      await save(objective.trim(), n);
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <>
      <Body>
        <div className="field">
          <label>{tr("system.goal.objective")}</label>
          <input type="text" data-testid="goal-objective" value={objective} onChange={(e) => setObjective(e.target.value)} />
        </div>
        <div className="field">
          <label>{tr("system.goal.maxRounds")}</label>
          <input type="text" inputMode="numeric" data-testid="goal-rounds" value={rounds} onChange={(e) => setRounds(e.target.value)} />
          <div className="hint">{tr(goal ? "system.goal.editHint" : "system.goal.setHint")}</div>
        </div>
        {error ? <div className="hint err" data-testid="goal-error">{error}</div> : null}
      </Body>
      <Foot>
        <button type="button" className="btn outline" onClick={cancel}>
          {tr("common.cancel")}
        </button>
        <button type="button" className="btn primary" data-testid="goal-save" onClick={() => void go()}>
          {tr("common.save")}
        </button>
      </Foot>
    </>
  );
}

function SessionOverview({ app, view, session }: { app: App; view: View; session: SessionKey }) {
  const tr = useT();
  const s = useShell(app);
  const params = useMemo(() => ({ instance: session.instance }), [session.instance]);
  const listed = useTopic<{ sessions: SessionRowView[] }>(view, "harnix.instances.sessions", params);
  const row = listed?.sessions?.find((r) => r.session_id === session.sessionId);
  const space = row?.space ?? "sessions";
  const agent: AgentRow | undefined = row ? { ...row, instance: session.instance, key: `${session.instance}/${row.session_id}` } : undefined;
  const turns = new Set(Object.keys(s.running));
  const state = agent ? stateOf(agent, turns) : null;
  const busy = Boolean(agent && (turns.has(agent.key) || agent.running || agent.tasks));
  const [round, setRound] = useState(0);
  // Read again whenever the row's pushed state says something happened, or an action here changed it.
  const facts = useAgentFacts(app, session, busy, `${round}|${agent ? signature(agent) : ""}|${JSON.stringify(row?.goal ?? null)}`);
  const own = facts.usage?.find((u) => u.session_id === session.sessionId);
  const subs = (facts.usage ?? []).filter((u) => u.session_id !== session.sessionId);
  const model = facts.context?.model ?? own?.models.join(", ") ?? "";
  return (
    <div className="system-pane" data-testid="system-session">
      <div className="agent-head">
        {state ? <span className={`dot ${state.dot}`} /> : null}
        <span className="nm">{row?.name || tr("sidebar.untitled")}</span>
        {row?.started_by === "harnix" ? <span className="kd own">Harnix</span> : null}
        {state ? <span className="kd" data-testid="agent-state">{state.label}</span> : null}
      </div>
      <Facts
        rows={[
          [tr("system.fact.instance"), session.instance],
          [tr("system.fact.session"), session.sessionId],
          [tr("system.fact.name"), row?.name ?? ""],
          [tr("system.fact.space"), space],
          [tr("system.fact.scene"), row?.scene ?? ""],
          [tr("system.fact.model"), model],
          [tr("system.fact.project"), row?.project_root ?? ""],
          [tr("system.fact.startedBy"), row?.started_by === "harnix" ? tr("system.startedBy.harnix") : tr("system.startedBy.user")],
        ]}
      />
      <GoalBlock app={app} session={session} goal={facts.goal} changed={() => setRound((n) => n + 1)} />
      <ContextRing context={facts.context} />
      <div className="label">{tr("system.stats")}</div>
      <Stats own={own} turns={facts.turns} />
      {subs.length ? (
        <div className="note" data-testid="agent-subs">
          {tr("system.subTotals", { count: subs.length, tokens: compact(subs.reduce((n, u) => n + totalTokens(u), 0)), time: ms(subs.reduce((n, u) => n + u.model_ms, 0)) })}
        </div>
      ) : null}
      {space !== SYSTEM ? (
        <button
          type="button"
          className="btn sm outline wide"
          data-testid="open-in-space"
          onClick={() => openWhereItBelongs(app, session, space)}
        >
          {tr("system.openInSpace", { space })}
        </button>
      ) : null}
      <button
        type="button"
        className="btn sm outline wide"
        data-testid="system-space-sessions"
        onClick={() => view.select({ kind: "space", space, current: session })}
      >
        {tr("system.spaceSessions", { space })}
      </button>
      <div className="label">{tr("system.ops")}</div>
      <Operations app={app} view={view} session={session} name={row?.name ?? null} />
    </div>
  );
}

/**
 * A sub-agent, picked: its own figures out of its parent's recording, how its task stands, and what
 * it has said — its transcript is a session of its own, read like any.
 */
function SubAgentOverview({ app, view, session, parent }: { app: App; view: View; session: SessionKey; parent: string }) {
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

function Operations({ app, view, session, name }: { app: App; view: View; session: SessionKey; name: string | null }) {
  const tr = useT();
  const target = { instance: session.instance, session_id: session.sessionId };
  const system = (method: string, extra: Record<string, unknown> = {}) => app.call(`harnix.system.${method}`, { ...target, ...extra });
  const claimed = (method: string, extra: Record<string, unknown> = {}) => app.call(method, { ...target, ...extra, space: SYSTEM });
  const settled = () => void app.refreshSessions();
  const ops: { id: string; icon: string; label: string; run: () => void; danger?: boolean }[] = [
    { id: "compact", icon: "archive", label: tr("system.op.compact"), run: () => void act(app, () => system("compact")).then(settled) },
    {
      id: "clear",
      icon: "refresh",
      label: tr("system.op.clear"),
      run: () =>
        confirmDialog(app, tr("system.confirmClear"), async () => {
          await system("clear");
          app.closeModal();
          settled();
        }),
    },
    {
      id: "fork",
      icon: "fork",
      label: tr("system.op.fork"),
      run: () =>
        void act(app, async () => {
          const forked = await app.call<{ session_id: string; instance?: string }>("session.fork", { ...target, space: SYSTEM });
          view.select({ kind: "session", instance: forked.instance ?? session.instance, sessionId: forked.session_id });
          settled();
        }),
    },
    {
      id: "model",
      icon: "agent",
      label: tr("system.op.model"),
      run: () => renameDialog(app, tr("system.op.model"), app.snapshot.hello?.engine?.model ?? "", (model) => system("model", { model: model.trim() })),
    },
    { id: "interrupt", icon: "stop", label: tr("system.op.interrupt"), run: () => void act(app, () => claimed("session.interrupt")) },
    { id: "export", icon: "download", label: tr("system.op.export"), run: () => exportDialog(app, { ...target, space: SYSTEM }) },
    { id: "rename", icon: "edit", label: tr("system.op.rename"), run: () => renameDialog(app, tr("system.op.rename"), name, (value) => system("rename", { name: value.trim() })) },
    { id: "close", icon: "close", label: tr("system.op.close"), run: () => void act(app, () => claimed("session.close")).then(settled) },
    {
      id: "delete",
      icon: "trash",
      label: tr("system.op.delete"),
      danger: true,
      run: () =>
        confirmDialog(app, tr("system.confirmDelete"), async () => {
          await claimed("session.delete");
          app.closeModal();
          app.forget(session);
          view.select({ kind: "instance", instance: session.instance });
          settled();
        }),
    },
  ];
  return (
    <div className="ops">
      {ops.map((op) => (
        <button type="button" key={op.id} className={`btn sm outline op${op.danger ? " danger" : ""}`} data-testid={`op-${op.id}`} onClick={op.run}>
          <Glyph name={op.icon} className="glyph" />
          {op.label}
        </button>
      ))}
    </div>
  );
}

/* ── the calls tab: every model call, what went in and what came out ──── */

interface TraceCall {
  index: number;
  turn: number;
  step: number;
  purpose?: string | null;
  model: string;
  input: { system: { source?: string | null; chars: number }[]; tools: number; messages: number };
  output: { text: string; chars: number; thinking_chars: number; tool_uses: number };
  tools: { id: string; name: string; input: string; result?: string | null; result_chars?: number | null; is_error?: boolean | null }[];
  outcome: { status: string; error?: string; message?: string };
  stop_reason?: string | null;
  usage?: { input_tokens?: number; output_tokens?: number; cache_read_input_tokens?: number; cache_creation_input_tokens?: number } | null;
  /** When it was sent, how long to its first chunk, how long it streamed; absent without the times. */
  timing?: { sent_at: number; ttft_ms?: number | null; generate_ms?: number | null; tokens_per_s?: number | null } | null;
  duration_ms?: number | null;
}

type Filter = "all" | "failed" | "tools";

function failed(call: TraceCall): boolean {
  return call.outcome?.status === "error" || call.tools.some((tool) => tool.is_error);
}

function CallsTab({ app, view, session }: { app: App; view: View; session: SessionKey }) {
  const tr = useT();
  // A jump from the conversation names a tool call: the call that asked for it is picked out.
  const [focus] = useState(() => view.takeTool());
  const [calls, setCalls] = useState<TraceCall[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const [range, setRange] = useState<[number, number] | null>(null);
  const [open, setOpen] = useState<number | null>(null);
  const list = useRef<HTMLDivElement>(null);
  const scroll = useRef(0);
  useEffect(() => {
    let alive = true;
    const load = () =>
      app
        .call<{ calls?: TraceCall[] }>("harnix.system.trace", { instance: session.instance, session_id: session.sessionId })
        .then((r) => alive && (setCalls(r.calls ?? []), setError(null)))
        .catch((e) => alive && setError(errorText(e)));
    void load();
    const off = app.client.onNotify((method, params) => {
      if (method === "harnix.turn_settled" && params.session_id === session.sessionId) void load();
    });
    return () => {
      alive = false;
      off();
    };
  }, [app, session.instance, session.sessionId]);
  useEffect(() => {
    if (open === null && list.current) list.current.scrollTop = scroll.current;
  }, [open]);
  useEffect(() => {
    if (focus && calls) list.current?.querySelector(`[data-tool="${CSS.escape(focus)}"]`)?.scrollIntoView({ block: "center" });
  }, [focus, calls]);
  if (error) return <div className="empty">{error}</div>;
  if (!calls) return null;
  if (open !== null) {
    const previous = calls.filter((c) => c.index < open && !c.purpose).pop() ?? null;
    return <CallDetail app={app} session={session} call={calls.find((c) => c.index === open)!} previous={previous} onBack={() => setOpen(null)} />;
  }
  if (!calls.length) return <div className="empty">{tr("system.noCalls")}</div>;
  const needle = query.trim().toLowerCase();
  const shown = calls.filter(
    (call) =>
      (filter === "failed" ? failed(call) : filter === "tools" ? call.tools.length > 0 : true) &&
      (!needle || searchable(call).includes(needle)) &&
      (!range || overlaps(call, range)),
  );
  const firstFailure = calls.find(failed);
  const turns = [...new Set(shown.map((c) => c.turn))];
  return (
    <div className="system-pane calls" data-testid="system-calls" ref={list}>
      <div className="tools-row">
        {(["all", "failed", "tools"] as Filter[]).map((f) => (
          <button type="button" key={f} className={`btn sm outline${filter === f ? " on" : ""}`} data-filter={f} onClick={() => setFilter(f)}>
            {tr(`system.filter.${f}`)}
          </button>
        ))}
        {firstFailure ? (
          <button
            type="button"
            className="btn sm outline end"
            data-testid="jump-to-failure"
            onClick={() => document.querySelector(`[data-call="${firstFailure.index}"]`)?.scrollIntoView({ block: "center" })}
          >
            {tr("system.jumpToFailure")}
          </button>
        ) : null}
      </div>
      <div className="tools-row">
        <input
          className="search"
          type="search"
          data-testid="calls-search"
          placeholder={tr("system.searchCalls")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        {range ? (
          <button type="button" className="btn sm outline end" data-testid="timeline-clear" onClick={() => setRange(null)}>
            {tr("system.clearRange")}
          </button>
        ) : null}
      </div>
      <Timeline calls={calls} range={range} onRange={setRange} />
      {!shown.length ? <div className="note" data-testid="calls-none">{tr("system.noMatchingCalls")}</div> : null}
      {turns.map((turn) => (
        <div key={turn}>
          <div className="turnh">{tr("system.turn", { turn })}</div>
          {shown
            .filter((c) => c.turn === turn)
            .map((call) => (
              <div key={call.index}>
                <button
                  type="button"
                  className={`step k-model${failed(call) && call.outcome?.status === "error" ? " bad" : ""}${focus && call.tools.some((tool) => tool.id === focus) ? " on" : ""}`}
                  data-call={call.index}
                  data-testid="system-call"
                  onClick={() => {
                    scroll.current = list.current?.scrollTop ?? 0;
                    setOpen(call.index);
                  }}
                >
                  <div className="l1">
                    <span className="kd2">{call.purpose ? call.purpose : tr("system.step.model")}</span>
                    <b>#{call.index + 1}</b>
                    <span className="r">
                      {call.model} · {tokens(call.usage)} · {call.duration_ms != null ? `${(call.duration_ms / 1000).toFixed(1)}s` : "—"}
                      {call.timing?.ttft_ms != null ? <span data-testid="call-ttft"> · {tr("system.ttft", { ms: call.timing.ttft_ms })}</span> : null}
                      {call.timing?.tokens_per_s != null ? ` · ${tr("system.rate", { rate: call.timing.tokens_per_s })}` : ""} · {call.stop_reason ?? call.outcome?.status}
                    </span>
                  </div>
                  <div className="l2">
                    <span className="in">{tr("system.callIn", { blocks: call.input.system.length, tools: call.input.tools, messages: call.input.messages })}</span>
                    <span className="arrow">→</span>
                    <span className="out">{callOut(call, tr)}</span>
                  </div>
                </button>
                {call.tools.map((tool) => (
                  <div key={tool.id} className={`step k-tool nested${tool.is_error ? " bad" : ""}${tool.id === focus ? " on" : ""}`} data-tool={tool.id}>
                    <div className="l1">
                      <span className="kd2">{tr("system.step.tool")}</span>
                      <b>{tool.name}</b>
                      <span className="r">{tool.result_chars != null ? `${tool.result_chars} chars` : tr("system.noResultYet")}</span>
                    </div>
                    <div className="l2">
                      <span className="in">{tool.input}</span>
                      <span className="arrow">→</span>
                      <span className="out">{tool.result ?? "…"}</span>
                    </div>
                  </div>
                ))}
              </div>
            ))}
        </div>
      ))}
    </div>
  );
}

/** What a search over the calls looks in: the answer, the model, what it was for, its tool calls. */
function searchable(call: TraceCall): string {
  return [call.output.text, call.model, call.purpose ?? "", ...call.tools.flatMap((tool) => [tool.name, tool.input, tool.result ?? ""])].join("\n").toLowerCase();
}

/** A call's stretch of time: from being sent to its end. */
function spanOf(call: TraceCall): [number, number] | null {
  const sent = call.timing?.sent_at;
  if (sent == null) return null;
  return [sent, sent + (call.duration_ms ?? 0)];
}

function overlaps(call: TraceCall, [from, to]: [number, number]): boolean {
  const span = spanOf(call);
  return span != null && span[0] <= to && span[1] >= from;
}

/**
 * The calls on one time line: each call a bar from when it was sent to its end, the wait for its
 * first chunk drawn lighter; between two calls of a turn, the tools ran. Dragging across it picks a
 * stretch of time, and the list below shows only the calls in it.
 */
function Timeline({ calls, range, onRange }: { calls: TraceCall[]; range: [number, number] | null; onRange: (r: [number, number] | null) => void }) {
  const tr = useT();
  const strip = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<[number, number] | null>(null);
  const spans = calls.map((call) => ({ call, span: spanOf(call) })).filter((x): x is { call: TraceCall; span: [number, number] } => x.span != null);
  if (!spans.length) return null;
  const start = Math.min(...spans.map((x) => x.span[0]));
  const end = Math.max(...spans.map((x) => x.span[1]));
  const total = Math.max(end - start, 1);
  const pct = (t: number) => `${((t - start) / total) * 100}%`;
  const width = (from: number, to: number) => `${(Math.max(to - from, 0) / total) * 100}%`;
  const at = (clientX: number) => {
    const box = strip.current!.getBoundingClientRect();
    return start + Math.min(Math.max((clientX - box.left) / box.width, 0), 1) * total;
  };
  const shownRange = drag ?? range;
  const gaps: { key: string; from: number; to: number }[] = [];
  for (let i = 1; i < spans.length; i += 1) {
    const a = spans[i - 1]!;
    const b = spans[i]!;
    if (a.call.turn === b.call.turn && !b.call.purpose && b.span[0] > a.span[1]) gaps.push({ key: `${a.call.index}-${b.call.index}`, from: a.span[1], to: b.span[0] });
  }
  return (
    <div className="timeline-wrap">
      <div
        className="timeline"
        ref={strip}
        data-testid="calls-timeline"
        title={tr("system.timelineHint")}
        onPointerDown={(e) => {
          strip.current?.setPointerCapture?.(e.pointerId);
          const t = at(e.clientX);
          setDrag([t, t]);
        }}
        onPointerMove={(e) => drag && setDrag([drag[0], at(e.clientX)])}
        onPointerUp={() => {
          if (!drag) return;
          const [a, b] = drag[0] <= drag[1] ? drag : [drag[1], drag[0]];
          setDrag(null);
          onRange(b - a >= total / 200 ? [a, b] : null);
        }}
      >
        {gaps.map((gap) => (
          <i key={gap.key} className="gap" style={{ left: pct(gap.from), width: width(gap.from, gap.to) }} />
        ))}
        {spans.map(({ call, span }) => (
          <i key={call.index} className={`call${call.purpose ? " aux" : ""}`} data-testid="timeline-call" data-call={call.index} style={{ left: pct(span[0]), width: width(span[0], span[1]) }}>
            {call.timing?.ttft_ms ? <b style={{ width: `${Math.min(100, (call.timing.ttft_ms / Math.max(span[1] - span[0], 1)) * 100)}%` }} /> : null}
          </i>
        ))}
        {shownRange ? (
          <i
            className="range"
            data-testid="timeline-range"
            style={{ left: pct(Math.min(...shownRange)), width: width(Math.min(...shownRange), Math.max(...shownRange)) }}
          />
        ) : null}
      </div>
      <div className="legend2">
        <span className="model">{tr("system.legend.model")}</span>
        <span className="wait">{tr("system.legend.wait")}</span>
        <span className="gap">{tr("system.legend.toolsTime")}</span>
        <span className="muted">{tr("system.duration", { seconds: (total / 1000).toFixed(1) })}</span>
      </div>
    </div>
  );
}

interface TurnRow {
  turn: number;
  calls: number[];
  auxiliary: number;
  duration_ms?: number | null;
  ttft_ms?: number | null;
  model_ms: number;
  tool_ms: number;
  input_tokens: number;
  output_tokens: number;
  cache_hit?: number | null;
  tokens_per_s?: number | null;
  context_tokens?: number | null;
  context_delta?: number | null;
}

/**
 * Per turn (DSH's statistics, for the system space): a row per turn — its calls, time to first
 * token, model and tool time, tokens, the cache's share by tokens, output rate, the context at its
 * end — opened into its calls. The context column shows the change from the turn before, or the
 * running total.
 */
function TurnsTab({ app, session }: { app: App; session: SessionKey }) {
  const tr = useT();
  const [data, setData] = useState<{ turns: TurnRow[]; calls: TraceCall[] } | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [mode, setMode] = useState<"delta" | "total">("delta");
  const [open, setOpen] = useState<Set<number>>(new Set());
  useEffect(() => {
    let alive = true;
    const target = { instance: session.instance, session_id: session.sessionId };
    const load = () =>
      Promise.all([app.call<{ turns?: TurnRow[] }>("harnix.system.turns", target), app.call<{ calls?: TraceCall[] }>("harnix.system.trace", target)])
        .then(([t, c]) => alive && (setData({ turns: t.turns ?? [], calls: c.calls ?? [] }), setFailure(null)))
        .catch((e) => alive && setFailure(errorText(e)));
    void load();
    const off = app.client.onNotify((method, params) => {
      if (method === "harnix.turn_settled" && params.session_id === session.sessionId) void load();
    });
    return () => {
      alive = false;
      off();
    };
  }, [app, session.instance, session.sessionId]);
  if (failure) return <div className="empty">{failure}</div>;
  if (!data) return null;
  if (!data.turns.length) return <div className="empty">{tr("system.noCalls")}</div>;
  const sec = (ms?: number | null) => (ms != null ? `${(ms / 1000).toFixed(1)}s` : "—");
  const pctOf = (x?: number | null) => (x != null ? `${Math.round(x * 100)}%` : "—");
  const signed = (n?: number | null) => (n == null ? "—" : n > 0 ? `+${compact(n)}` : n < 0 ? `−${compact(-n)}` : "0");
  const toggle = (turn: number) => setOpen((was) => {
    const next = new Set(was);
    if (next.has(turn)) next.delete(turn);
    else next.add(turn);
    return next;
  });
  return (
    <div className="system-pane turns" data-testid="system-turns">
      <div className="tools-row">
        {(["delta", "total"] as const).map((m) => (
          <button type="button" key={m} className={`btn sm outline${mode === m ? " on" : ""}`} data-testid={`turns-${m}`} onClick={() => setMode(m)}>
            {tr(`system.turns.${m}`)}
          </button>
        ))}
      </div>
      <table className="frames">
        <thead>
          <tr>
            <th>{tr("system.turns.turn")}</th>
            <th>{tr("system.turns.calls")}</th>
            <th>TTFT</th>
            <th>{tr("system.turns.model")}</th>
            <th>{tr("system.turns.tools")}</th>
            <th>{tr("system.turns.inOut")}</th>
            <th>{tr("system.turns.cache")}</th>
            <th>{tr("system.turns.rate")}</th>
            <th>{mode === "delta" ? tr("system.turns.contextDelta") : tr("system.turns.context")}</th>
          </tr>
        </thead>
        <tbody>
          {data.turns.map((row) => (
            <Fragment key={row.turn}>
              <tr className="turn-row" data-testid="turn-row" data-turn={row.turn} onClick={() => toggle(row.turn)}>
                <td>{row.turn}</td>
                <td>
                  {row.calls.length}
                  {row.auxiliary ? <span className="muted"> +{row.auxiliary}</span> : null}
                </td>
                <td>{sec(row.ttft_ms)}</td>
                <td>{sec(row.model_ms)}</td>
                <td>{sec(row.tool_ms)}</td>
                <td>
                  {compact(row.input_tokens)} / {compact(row.output_tokens)}
                </td>
                <td data-testid="turn-cache">{pctOf(row.cache_hit)}</td>
                <td>{row.tokens_per_s != null ? row.tokens_per_s : "—"}</td>
                <td data-testid="turn-context">{mode === "delta" ? signed(row.context_delta) : row.context_tokens != null ? compact(row.context_tokens) : "—"}</td>
              </tr>
              {open.has(row.turn)
                ? row.calls.map((index) => {
                    const call = data.calls.find((c) => c.index === index);
                    if (!call) return null;
                    return (
                      <tr key={index} className="step-row" data-testid="turn-step" data-call={index}>
                        <td />
                        <td>#{index + 1}</td>
                        <td>{sec(call.timing?.ttft_ms)}</td>
                        <td>{sec(call.duration_ms)}</td>
                        <td>{call.tools.map((tool) => tool.name).join(", ") || "—"}</td>
                        <td colSpan={2}>{tokens(call.usage)}</td>
                        <td>{call.timing?.tokens_per_s ?? "—"}</td>
                        <td />
                      </tr>
                    );
                  })
                : null}
            </Fragment>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function tokens(usage: TraceCall["usage"]): string {
  if (!usage) return "—";
  const input = (usage.input_tokens ?? 0) + (usage.cache_read_input_tokens ?? 0) + (usage.cache_creation_input_tokens ?? 0);
  return `in ${compact(input)} · out ${compact(usage.output_tokens ?? 0)}`;
}

function compact(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
}

function callOut(call: TraceCall, tr: (k: string, v?: Record<string, unknown>) => string): string {
  if (call.outcome?.status === "error") return `${tr("system.error")}: ${call.outcome.message ?? call.outcome.error ?? ""}`;
  const tools = call.tools.map((tool) => `tool_use: ${tool.name}`).join(", ");
  return [clip(call.output.text, 80), tools].filter(Boolean).join(" · ") || "—";
}

interface CallDetailData {
  input: {
    system: { source?: string | null; origin?: unknown; text?: string }[];
    tools: { name: string; description?: string; input_schema?: unknown }[];
    messages: { role: string; content: unknown }[];
    model: string;
    max_tokens: number;
    thinking_mode?: unknown;
    provider?: string;
  };
  output: {
    text: string;
    thinking_chars: number;
    tool_uses: { id: string; name: string; input: unknown }[];
    outcome: unknown;
    stop_reason?: string | null;
    usage?: unknown;
    duration_ms?: number | null;
  };
}

/** One call in full, inside the right column: what went in, what came out, what changed. */
function CallDetail({ app, session, call, previous, onBack }: { app: App; session: SessionKey; call: TraceCall; previous: TraceCall | null; onBack: () => void }) {
  const tr = useT();
  const [side, setSide] = useState<"input" | "output">("input");
  const [data, setData] = useState<CallDetailData | null>(null);
  const [before, setBefore] = useState<CallDetailData | null>(null);
  const [compare, setCompare] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const target = { instance: session.instance, session_id: session.sessionId };
    void app
      .call<CallDetailData>("harnix.system.trace.call", { ...target, index: call.index })
      .then(setData)
      .catch((e) => setError(errorText(e)));
    if (previous)
      void app
        .call<CallDetailData>("harnix.system.trace.call", { ...target, index: previous.index })
        .then(setBefore)
        .catch(() => undefined);
  }, [app, session.instance, session.sessionId, call.index, previous?.index]);
  const copy = () => void navigator.clipboard?.writeText(JSON.stringify(data, null, 2)).catch(() => undefined);
  return (
    <div className="system-pane call-detail" data-testid="call-detail">
      <div className="cd-h">
        <button type="button" className="ib back" data-testid="call-back" title={tr("common.back")} onClick={onBack}>
          <Glyph name="chevronRight" className="glyph flip" />
        </button>
        <b>#{call.index + 1}</b>
        <span className="muted">{call.model}</span>
        <span className="sp" />
        {previous ? (
          <button type="button" className={`btn sm outline${compare ? " on" : ""}`} data-testid="call-compare" onClick={() => setCompare(!compare)}>
            {tr("system.compare")}
          </button>
        ) : null}
        <button type="button" className="btn sm outline" onClick={copy}>
          {tr("system.copyJson")}
        </button>
        <button type="button" className="btn sm outline" data-testid="call-locate" onClick={() => void locateInConversation(app, session, call.tools.at(0)?.id ?? null)}>
          {tr("system.locateInConversation")}
        </button>
      </div>
      <div className="cd-stats">
        {tokens(call.usage)} · {call.duration_ms != null ? `${(call.duration_ms / 1000).toFixed(1)}s` : "—"} · {call.stop_reason ?? call.outcome?.status}
      </div>
      <div className="stabs" role="tablist">
        {(["input", "output"] as const).map((s) => (
          <button type="button" role="tab" key={s} data-side={s} aria-selected={side === s} className={side === s ? "on" : ""} onClick={() => setSide(s)}>
            {tr(`system.side.${s}`)}
          </button>
        ))}
      </div>
      {error ? <div className="empty">{error}</div> : null}
      {data && compare && before ? <CallDiff before={before} after={data} /> : null}
      {data && side === "input" ? (
        <div data-testid="call-input">
          <div className="label">{tr("system.blocks", { count: data.input.system.length })}</div>
          {data.input.system.map((block, i) => (
            <Disclosure key={i} testId="call-block" state="ok" summary={<span className="hx-tool-name">{block.source ?? `#${i + 1}`}</span>} tail={`${(block.text ?? "").length} chars`}>
              <pre className="hx-pre">{block.text}</pre>
            </Disclosure>
          ))}
          <div className="label">{tr("system.toolsCount", { count: data.input.tools.length })}</div>
          <div className="note">{data.input.tools.map((tool) => tool.name).join(", ") || tr("request.noTools")}</div>
          <div className="label">{tr("system.messagesCount", { count: data.input.messages.length })}</div>
          {data.input.messages.map((message, i) => (
            <Disclosure key={i} testId="call-message" state="ok" summary={<><span className="hx-tool-name">{i}</span><span className="hx-tool-arg">{message.role}</span></>} tail={clip(messageText(message.content), 60)}>
              <pre className="hx-pre">{JSON.stringify(message.content, null, 2)}</pre>
            </Disclosure>
          ))}
          <div className="label">{tr("system.config")}</div>
          <pre className="hx-pre">{JSON.stringify({ model: data.input.model, max_tokens: data.input.max_tokens, thinking_mode: data.input.thinking_mode, provider: data.input.provider }, null, 2)}</pre>
        </div>
      ) : null}
      {data && side === "output" ? (
        <div data-testid="call-output">
          {data.output.text ? <div className="otext">{data.output.text}</div> : null}
          {data.output.thinking_chars ? <div className="note">{tr("system.thinking", { chars: data.output.thinking_chars })}</div> : null}
          {data.output.tool_uses.map((tool) => (
            <div key={tool.id}>
              <div className="label">tool_use · {tool.name}</div>
              <pre className="hx-pre">{JSON.stringify(tool.input, null, 2)}</pre>
            </div>
          ))}
          {call.tools.map((tool) => (
            <div key={tool.id} className={tool.is_error ? "bad" : ""}>
              <div className="label">
                tool_result · {tool.name} {tool.is_error ? `· ${tr("system.error")}` : ""}
              </div>
              <pre className="hx-pre scroll">{tool.result ?? tr("system.noResultYet")}</pre>
            </div>
          ))}
          <div className="label">{tr("system.end")}</div>
          <pre className="hx-pre">{JSON.stringify({ outcome: data.output.outcome, stop_reason: data.output.stop_reason, usage: data.output.usage, duration_ms: data.output.duration_ms }, null, 2)}</pre>
        </div>
      ) : null}
    </div>
  );
}

/** What differs from the call before: system blocks, tools, and how many messages. */
function CallDiff({ before, after }: { before: CallDetailData; after: CallDetailData }) {
  const tr = useT();
  const lines: { sign: "+" | "-" | "~"; text: string }[] = [];
  const blocks = (d: CallDetailData) => new Map(d.input.system.map((b, i) => [b.source ?? `#${i + 1}`, b.text ?? ""]));
  const was = blocks(before);
  const is = blocks(after);
  for (const [name, text] of is) {
    if (!was.has(name)) lines.push({ sign: "+", text: tr("system.diff.block", { label: name, chars: text.length }) });
    else if (was.get(name) !== text) lines.push({ sign: "~", text: tr("system.diff.changed", { label: name, from: was.get(name)!.length, to: text.length }) });
  }
  for (const [name, text] of was) if (!is.has(name)) lines.push({ sign: "-", text: tr("system.diff.block", { label: name, chars: text.length }) });
  const had = new Set(before.input.tools.map((tool) => tool.name));
  const has = new Set(after.input.tools.map((tool) => tool.name));
  for (const name of has) if (!had.has(name)) lines.push({ sign: "+", text: tr("system.diff.tool", { name }) });
  for (const name of had) if (!has.has(name)) lines.push({ sign: "-", text: tr("system.diff.tool", { name }) });
  const added = after.input.messages.length - before.input.messages.length;
  if (added) lines.push({ sign: added > 0 ? "+" : "-", text: tr("system.diff.messages", { count: Math.abs(added) }) });
  if (!lines.length) return <div className="diff none">{tr("system.diff.same")}</div>;
  return (
    <div className="diff" data-testid="system-diff">
      {lines.map((line, i) => (
        <div key={i} className={`line ${line.sign === "+" ? "add" : line.sign === "-" ? "del" : "mod"}`}>
          <span className="sign">{line.sign}</span>
          {line.text}
        </div>
      ))}
    </div>
  );
}

/* ── the context tab: what the next call carries, and the messages ─────── */

interface Context {
  model?: string;
  max_tokens?: number;
  system?: { name: string; origin: unknown; tokens: number; text: string }[];
  tools?: { name: string; source?: string; tokens?: number }[];
  messages?: { count: number; tokens: number };
  total_tokens?: number;
  compact_threshold?: number;
  /** What the provider counted for the last call made; absent before the first. */
  measured?: { tokens: number; window?: number; compact_threshold?: number };
}

interface HistoryMessage {
  seq: number;
  role: string;
  origin?: string | null;
  content: unknown;
}

function ContextTab({ app, view, session }: { app: App; view: View; session: SessionKey }) {
  const tr = useT();
  const [context, setContext] = useState<Context | null>(null);
  const [history, setHistory] = useState<{ messages: HistoryMessage[]; total: number } | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [round, setRound] = useState(0);
  const target = { instance: session.instance, session_id: session.sessionId };
  useEffect(() => {
    let alive = true;
    const load = async () => {
      try {
        const [ctx, first] = await Promise.all([
          app.call<Context>("harnix.system.context", target),
          app.call<{ total: number }>("session.history", { ...target, space: SYSTEM, offset: 0, limit: 1 }),
        ]);
        const offset = Math.max(0, first.total - 50);
        const page = await app.call<{ messages: HistoryMessage[]; total: number }>("session.history", { ...target, space: SYSTEM, offset, limit: 50 });
        if (!alive) return;
        setContext(ctx);
        setHistory({ messages: page.messages ?? [], total: page.total });
        setFailure(null);
      } catch (e) {
        if (alive) setFailure(errorText(e));
      }
    };
    void load();
    const off = app.client.onNotify((method, params) => {
      if (method === "harnix.turn_settled" && params.session_id === session.sessionId) void load();
    });
    return () => {
      alive = false;
      off();
    };
  }, [app, session.instance, session.sessionId, round]);
  if (failure) return <div className="empty" data-testid="system-context-failed">{failure}</div>;
  if (!context) return null;
  const total = history?.total ?? 0;
  const again = () => setRound((n) => n + 1);
  const systemTokens = (context.system ?? []).reduce((n, b) => n + b.tokens, 0);
  const toolTokens = (context.tools ?? []).reduce((n, b) => n + (b.tokens ?? 0), 0);
  const messageTokens = context.messages?.tokens ?? 0;
  // The bar is as long as the provider's own count of the last call (when there is one), against the
  // window; the estimates only divide it. An estimate is never shown as if it were a count: `~`.
  const estimated = Math.max(systemTokens + toolTokens + messageTokens, 1);
  const measured = context.measured?.tokens ?? null;
  const threshold = context.measured?.compact_threshold ?? context.compact_threshold ?? 0;
  const length = measured ?? estimated;
  const budget = Math.max(context.measured?.window ?? 0, threshold, length, 1);
  const pct = (n: number) => `${Math.min(100, ((n / estimated) * length * 100) / budget)}%`;
  const about = (n: number) => `~${n}`;
  return (
    <div className="system-pane" data-testid="system-context">
      <div className="note">{tr("system.contextNote")}</div>
      <div className="stack" data-testid="context-stack" data-measured={measured ?? undefined} title={measured != null ? tr("system.tokens", { count: measured }) : tr("system.tokens", { count: about(estimated) })}>
        <i className="sys" style={{ width: pct(systemTokens) }} />
        <i className="tools" style={{ width: pct(toolTokens) }} />
        <i className="msgs" style={{ width: pct(messageTokens) }} />
      </div>
      <div className="legend2">
        <span className="sys">{tr("system.legend.system", { count: about(systemTokens) })}</span>
        <span className="tools">{tr("system.legend.tools", { count: about(toolTokens) })}</span>
        <span className="msgs">{tr("system.legend.messages", { count: about(messageTokens) })}</span>
      </div>
      <Facts
        rows={[
          [tr("system.fact.model"), context.model ?? ""],
          [
            tr("system.fact.measured"),
            measured != null
              ? context.measured?.window
                ? tr("system.tokensOf", { count: measured, of: context.measured.window })
                : tr("system.tokens", { count: measured })
              : tr("system.notMeasured"),
          ],
          [tr("system.fact.estimated"), tr("system.tokens", { count: about(context.total_tokens ?? estimated) })],
          [tr("system.fact.threshold"), threshold ? tr("system.tokens", { count: threshold }) : ""],
          [tr("system.fact.messages"), tr("system.messagesTokens", { count: context.messages?.count ?? 0, tokens: about(context.messages?.tokens ?? 0) })],
        ]}
      />
      <div className="label">{tr("system.blocks", { count: context.system?.length ?? 0 })}</div>
      {(context.system ?? []).map((block, i) => (
        <Disclosure
          key={i}
          testId="system-block"
          state="ok"
          summary={
            <>
              <span className="hx-tool-name">{block.name}</span>
              <span className="hx-tool-arg">{originLabel(block.origin)}</span>
            </>
          }
          tail={tr("system.tokens", { count: about(block.tokens) })}
        >
          <pre className="hx-pre">{block.text}</pre>
        </Disclosure>
      ))}
      <div className="label">{tr("system.toolsCount", { count: context.tools?.length ?? 0 })}</div>
      <div className="note">{(context.tools ?? []).map((tool) => tool.name).join(", ") || tr("request.noTools")}</div>
      <div className="label">{tr("system.messages")}</div>
      <div className="messages" data-testid="system-messages">
        {(history?.messages ?? []).map((message) => {
          const origin = message.origin ?? message.role;
          const text = messageText(message.content);
          const droppable = origin === "user" || origin === "steer" || origin === "command" || origin === "tool_result";
          const editable = (origin === "user" || origin === "steer") && isPlainText(message.content);
          return (
            <div key={message.seq} className="message" data-seq={message.seq} data-origin={origin}>
              <span className="seq">{message.seq}</span>
              <span className="origin">{origin}</span>
              <span className="text truncate" title={text}>
                {clip(text, 160)}
              </span>
              {editable ? (
                <button
                  type="button"
                  className="icon-btn"
                  data-testid="message-edit"
                  title={tr("system.editMessage")}
                  onClick={() =>
                    renameDialog(app, tr("system.editMessage"), text, async (value) => {
                      await app.call("harnix.system.messages.edit", { ...target, at: message.seq, text: value, total });
                      again();
                    })
                  }
                >
                  <Glyph name="edit" className="glyph" />
                </button>
              ) : null}
              <button
                type="button"
                className="icon-btn"
                data-testid="message-fork"
                title={tr("system.forkHere")}
                onClick={() =>
                  void act(app, async () => {
                    // `at_message` is how many to keep: this one and everything before it.
                    const forked = await app.call<{ session_id: string; instance?: string }>("session.fork", { ...target, at_message: message.seq + 1, space: SYSTEM });
                    void app.refreshSessions();
                    view.select({ kind: "session", instance: forked.instance ?? session.instance, sessionId: forked.session_id });
                  })
                }
              >
                <Glyph name="fork" className="glyph" />
              </button>
              {droppable ? (
                <button
                  type="button"
                  className="icon-btn"
                  data-testid="message-drop"
                  title={tr("system.dropMessage")}
                  onClick={() =>
                    void act(app, async () => {
                      await app.call("harnix.system.messages.drop", { ...target, at: [message.seq], total });
                      again();
                    })
                  }
                >
                  <Glyph name="trash" className="glyph" />
                </button>
              ) : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function originLabel(origin: unknown): string {
  if (typeof origin === "string") return origin;
  if (origin && typeof origin === "object") {
    const [kind, value] = Object.entries(origin as Record<string, unknown>)[0] ?? [];
    return kind ? `${kind}:${String(value)}` : "";
  }
  return "";
}

function messageText(content: unknown): string {
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

function isPlainText(content: unknown): boolean {
  return typeof content === "string" || (Array.isArray(content) && content.every((b: { type?: string }) => b.type === "text"));
}

/* ── the tasks tab ────────────────────────────────────────────────────── */

function TasksTab({ app, view, session }: { app: App; view: View; session: SessionKey }) {
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
          <div className="task-h" onClick={() => view.select({ kind: "task", instance: session.instance, sessionId: session.sessionId, taskId: task.task_id })}>
            <span className={`dot ${task.status}`} />
            <span className="kind">{tr(`system.task.${task.kind}`)}</span>
            <span className="what truncate" title={taskLabel(task)}>
              {taskLabel(task)}
            </span>
            <span className="when">{duration(task)}</span>
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
function TaskMessage({ app, params, task }: { app: App; params: { instance: string; session_id: string }; task: Task }) {
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
      setSaid({ ok: false, text: errorText(e) });
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

/** A session's teams (Agix `team_progress`, as the hub last heard it): each one's stage and members,
 *  every member with how its task stands. */
interface Team {
  team: string;
  team_id: string;
  stage: string;
  stage_index: number;
  stage_count: number;
  status: string;
  members: string[];
  failed: string[];
}

function TeamBoard({ teams, tasks }: { teams: Team[]; tasks: Task[] }) {
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
              <span className="note">{Object.entries(counts).map(([status, n]) => `${tr(`system.taskStatus.${status}`)} ${n}`).join(" · ")}</span>
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

/** How long the first press of "stop" waits for the second (DSH's `KILL_ARM_MS`). */
const STOP_ARM_MS = 3000;

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

/** The most of a task's output this page keeps: the engine keeps the last 256 KiB. */
const OUTPUT_KEPT = 256 * 1024;

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
    [tr("system.fact.duration"), duration(task)],
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

function taskLabel(task: Task): string {
  const d = task.detail ?? {};
  const text = d.description ?? d.command ?? d.prompt ?? d.member ?? d.cron ?? task.task_id;
  return clip(String(text), 120);
}

function duration(task: Task): string {
  if (!task.started_ms) return "";
  const secs = Math.max(0, Math.round(((task.ended_ms ?? Date.now()) - task.started_ms) / 1000));
  return secs < 60 ? `${secs}s` : `${Math.floor(secs / 60)}m${String(secs % 60).padStart(2, "0")}s`;
}

/* ── the frames tab: the session's frames as they arrive ──────────────── */

interface FrameRow {
  at: number;
  seq?: number;
  kind: string;
  frame: Record<string, unknown>;
}

/**
 * Watching a session here is `harnix.attach` like any viewer's, claiming the system space: the
 * frames that arrive are the ones every viewer gets, in order, with their `seq`. Leaving the tab
 * detaches, unless this page has the session open somewhere else.
 */
function FramesTab({ app, session }: { app: App; session: SessionKey }) {
  const tr = useT();
  const [rows, setRows] = useState<FrameRow[]>([]);
  const [paused, setPaused] = useState(false);
  const [kind, setKind] = useState("");
  const pausedRef = useRef(paused);
  pausedRef.current = paused;
  useEffect(() => {
    const target = { instance: session.instance, session_id: session.sessionId };
    let alive = true;
    const off = app.client.onNotify((method, params) => {
      if (!alive || pausedRef.current || params.session_id !== session.sessionId) return;
      if (params.instance !== undefined && params.instance !== session.instance) return;
      const event = (params.event ?? {}) as { kind?: string };
      const row: FrameRow = { at: Date.now(), seq: params.seq as number | undefined, kind: event.kind ?? method, frame: { method, params } };
      setRows((now) => [...now.slice(-499), row]);
    });
    void app.call("harnix.attach", { ...target, space: SYSTEM }).catch(() => undefined);
    return () => {
      alive = false;
      off();
      const open = Object.values(app.snapshot.currentBySpace).some((k) => k && keyOf(k.instance, k.sessionId) === keyOf(session.instance, session.sessionId));
      if (!open) void app.call("harnix.detach", target).catch(() => undefined);
    };
  }, [app, session.instance, session.sessionId]);
  const kinds = [...new Set(rows.map((r) => r.kind))];
  const shown = kind ? rows.filter((r) => r.kind === kind) : rows;
  return (
    <div className="system-pane" data-testid="system-frames">
      <div className="tools-row">
        <select value={kind} onChange={(e) => setKind(e.target.value)} data-testid="frames-kind">
          <option value="">{tr("system.frames.all")}</option>
          {kinds.map((k) => (
            <option key={k} value={k}>
              {k}
            </option>
          ))}
        </select>
        <button type="button" className={`btn sm outline${paused ? " on" : ""}`} onClick={() => setPaused(!paused)}>
          {paused ? tr("system.frames.resume") : tr("system.frames.pause")}
        </button>
        <button type="button" className="btn sm outline" onClick={() => void navigator.clipboard?.writeText(JSON.stringify(shown.map((r) => r.frame), null, 2)).catch(() => undefined)}>
          {tr("system.copyJson")}
        </button>
      </div>
      {!shown.length ? <div className="note">{tr("system.frames.waiting")}</div> : null}
      <table className="frames">
        <tbody>
          {shown
            .slice()
            .reverse()
            .map((row, i) => (
              <tr key={`${row.at}-${i}`} data-kind={row.kind}>
                <td>{row.seq ?? ""}</td>
                <td>{row.kind}</td>
                <td>{since(row.at)}</td>
                <td className="truncate">{clip(JSON.stringify(row.frame.params), 140)}</td>
              </tr>
            ))}
        </tbody>
      </table>
    </div>
  );
}
