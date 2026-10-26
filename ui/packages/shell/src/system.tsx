/**
 * The system space (design §9): what this process runs, and the hands to act on it.
 *
 * Its three columns: on the left, the instances; in the centre, the console — the process at a
 * glance, then what is running right now (each instance, its running sessions, their running
 * tasks; what finishes fades out) and the diagnostics; on the right, whatever was picked there:
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

import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { errorText } from "@harnix/core";
import { Disclosure, clip, t, useT, type SpaceTab } from "@harnix/react";

import { keyOf, type App, type SessionKey } from "./app.ts";
import { confirmDialog, Connections, renameDialog, since, type Connection } from "./dialogs.tsx";
import { Glyph } from "./glyph.tsx";
import { useShell } from "./hooks.ts";
import { Diagnostics } from "./settings.tsx";

export const SYSTEM = "system";
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
  project_root?: string | null;
  space?: string;
  last_active?: string;
}

interface Task {
  task_id: string;
  kind: string;
  status: string;
  started_ms?: number | null;
  ended_ms?: number | null;
  detail?: Record<string, unknown>;
  error?: string;
}

/** What the right column is about. */
export type Selection =
  | { kind: "instance"; instance: string }
  | { kind: "session"; instance: string; sessionId: string }
  | { kind: "task"; instance: string; sessionId: string; taskId: string };

export function registerSystem(app: App): void {
  if (!app.snapshot.reachable.includes("harnix.system.context")) return;
  const view = new View(app);
  app.registry.register(
    "space",
    {
      id: SYSTEM,
      icon: "cpu",
      order: 90,
      title: () => t("space.system"),
      subtitle: () => t("space.system.subtitle"),
      center: "hidden",
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
  if (picked.kind === "instance") {
    return [{ id: "system.overview", title: () => t("system.tab.overview"), render: () => <InstanceOverview app={app} view={view} instance={picked.instance} /> }];
  }
  const session: SessionKey = { instance: picked.instance, sessionId: picked.sessionId };
  const tabs: SpaceTab[] = [
    { id: "system.overview", title: () => t("system.tab.overview"), render: () => <SessionOverview app={app} view={view} session={session} /> },
    { id: "system.calls", title: () => t("system.tab.calls"), render: () => <CallsTab app={app} session={session} /> },
    { id: "system.context", title: () => t("system.tab.context"), render: () => <ContextTab app={app} session={session} /> },
    { id: "system.tasks", title: () => t("system.tab.tasks"), render: () => <TasksTab app={app} view={view} session={session} /> },
    { id: "system.frames", title: () => t("system.tab.frames"), render: () => <FramesTab app={app} session={session} /> },
  ];
  // A task opens on its own tab.
  if (picked.kind === "task") tabs.unshift(tabs.splice(3, 1)[0]!);
  return tabs;
}

/* ── what is selected, and the topics that keep the view current ──────── */

/**
 * The selection, and the hub topics this space watches. A topic is subscribed once however many
 * parts of the page read it, and given up when the last one stops.
 */
class View {
  private picked: Selection | null = null;
  private readonly listeners = new Set<() => void>();
  private readonly topics = new Map<string, { value: unknown; readers: number; listeners: Set<() => void> }>();

  constructor(private readonly app: App) {
    app.client.onNotify((method, params) => {
      if (method !== "harnix.topic") return;
      const entry = this.topics.get(topicKey(String(params.topic), params.params));
      if (!entry) return;
      entry.value = params.value;
      for (const fn of [...entry.listeners]) fn();
    });
    // A reconnect forgets every subscription: take them out again.
    app.client.onStatus((status) => {
      if (status !== "open") return;
      for (const key of this.topics.keys()) void this.fetch(key);
    });
  }

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  readonly selected = (): Selection | null => this.picked;

  select(selection: Selection): void {
    this.picked = selection;
    for (const fn of [...this.listeners]) fn();
    // The right column is where a selection is shown; redraw it with the new tabs.
    this.app.set({ detail: null, detailsOpen: true });
  }

  watch(topic: string, params: Record<string, unknown> | undefined, fn: () => void): () => void {
    const key = topicKey(topic, params);
    let entry = this.topics.get(key);
    if (!entry) {
      entry = { value: undefined, readers: 0, listeners: new Set() };
      this.topics.set(key, entry);
      void this.fetch(key);
    }
    entry.readers += 1;
    entry.listeners.add(fn);
    return () => {
      entry!.listeners.delete(fn);
      entry!.readers -= 1;
      if (entry!.readers === 0) {
        this.topics.delete(key);
        void this.app.client.call("harnix.unsubscribe", { topic, ...(params ? { params } : {}) }).catch(() => undefined);
      }
    };
  }

  value(topic: string, params?: Record<string, unknown>): unknown {
    return this.topics.get(topicKey(topic, params))?.value;
  }

  private async fetch(key: string): Promise<void> {
    const { topic, params } = JSON.parse(key) as { topic: string; params?: Record<string, unknown> };
    try {
      const answer = (await this.app.client.call("harnix.subscribe", { topic, ...(params ? { params } : {}) })) as { value: unknown };
      const entry = this.topics.get(key);
      if (!entry) return;
      entry.value = answer.value;
      for (const fn of [...entry.listeners]) fn();
    } catch {
      /* the instance is down or gone: the tree says so from `harnix.instances` */
    }
  }
}

function topicKey(topic: string, params: unknown): string {
  return JSON.stringify(params == null ? { topic } : { topic, params });
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
  if (!picked || picked.kind !== row.kind || picked.instance !== row.instance) return false;
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
  connections?: Connection[];
}

function Console({ app, view }: { app: App; view: View }) {
  const tr = useT();
  const [tab, setTab] = useState<"running" | "diagnostics">("running");
  return (
    <div className="system-console" data-testid="system-console">
      <Strip app={app} view={view} />
      <div className="stabs" role="tablist">
        <button type="button" role="tab" data-tab="running" aria-selected={tab === "running"} className={tab === "running" ? "on" : ""} onClick={() => setTab("running")}>
          {tr("system.running")}
        </button>
        <button type="button" role="tab" data-tab="diagnostics" aria-selected={tab === "diagnostics"} className={tab === "diagnostics" ? "on" : ""} onClick={() => setTab("diagnostics")}>
          {tr("system.diagnostics")}
        </button>
      </div>
      {tab === "running" ? <Running app={app} view={view} /> : <div className="system-diagnostics settings-page"><Diagnostics app={app} /></div>}
    </div>
  );
}

/** The process in one row: versions, how long it has run, its memory, who is connected, and what runs. */
function Strip({ app, view }: { app: App; view: View }) {
  const tr = useT();
  const s = useShell(app);
  const [process, setProcess] = useState<Process | null>(null);
  useEffect(() => {
    let alive = true;
    const load = () => void app.call<Process>("harnix.process", {}).then((p) => alive && setProcess(p)).catch(() => undefined);
    load();
    const timer = setInterval(load, 5000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [app]);
  const instances = useTopic<{ instances: InstanceRow[] }>(view, "harnix.instances");
  const tasks = useRunningTasks(view, Object.keys(s.running));
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
      {kpi("uptime", process?.uptime_secs != null ? uptime(process.uptime_secs) : "—", tr("system.uptime"))}
      {kpi("memory", process?.peak_rss_bytes ? `${Math.round(process.peak_rss_bytes / 1048576)} MB` : "—", tr("system.peakMemory"))}
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
      {kpi("sessions", String(Object.keys(s.running).length), tr("system.runningSessions"))}
      {kpi("tasks", String(tasks), tr("system.runningTasks"))}
    </div>
  );
}

function uptime(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return h ? `${h}:${String(m).padStart(2, "0")}:${String(secs % 60).padStart(2, "0")}` : `${m}:${String(secs % 60).padStart(2, "0")}`;
}

const LIVE = new Set(["running", "waiting", "pending"]);

/** How many tasks are running across these sessions (`instance/session` keys). */
function useRunningTasks(view: View, keys: string[]): number {
  const [, bump] = useState(0);
  const joined = keys.sort().join(",");
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
 * What is running right now: each instance, its running sessions, their running tasks. Something
 * that stops running stays a moment, fading, and is gone — finished work is not listed here.
 */
function Running({ app, view }: { app: App; view: View }) {
  const tr = useT();
  const listed = useTopic<{ instances: InstanceRow[] }>(view, "harnix.instances");
  const s = useShell(app);
  const picked = useSelection(view);
  const running = Object.keys(s.running);
  const leaving = useLeaving(running);
  return (
    <div className="tree2" data-testid="system-running">
      {(listed?.instances ?? []).map((instance) => {
        const mine = [...new Set([...running, ...leaving])].filter((key) => key.startsWith(`${instance.id}/`));
        return (
          <div key={instance.id} className="tgroup" data-instance={instance.id}>
            <div
              className={`tr inst${isSelected(picked, { kind: "instance", instance: instance.id }) ? " on" : ""}`}
              data-state={instance.status.state}
              onClick={() => view.select({ kind: "instance", instance: instance.id })}
            >
              <span className={`dot ${instance.status.state}`} />
              <span className="nm">{instance.id}</span>
              <span className="kd">{tr(`system.carrier.${instance.carrier}`)}</span>
              <span className="kd">{instance.status.state}</span>
              <span className="act">{running.filter((k) => k.startsWith(`${instance.id}/`)).length ? tr("system.runningIn", { count: running.filter((k) => k.startsWith(`${instance.id}/`)).length }) : tr("system.idle")}</span>
            </div>
            {mine.map((key) => (
              <RunningSession key={key} app={app} view={view} instance={instance.id} sessionId={splitKey(key)[1]} done={!running.includes(key)} />
            ))}
          </div>
        );
      })}
      <div className="note">{tr("system.runningNote")}</div>
    </div>
  );
}

/** Keys that just stopped being in `live`, for as long as they take to fade. */
function useLeaving(live: string[]): string[] {
  const previous = useRef<string[]>(live);
  const [leaving, setLeaving] = useState<string[]>([]);
  const joined = [...live].sort().join(",");
  useEffect(() => {
    const gone = previous.current.filter((key) => !live.includes(key));
    previous.current = live;
    if (!gone.length) return;
    setLeaving((now) => [...now, ...gone]);
    const timer = setTimeout(() => setLeaving((now) => now.filter((key) => !gone.includes(key))), 1400);
    return () => clearTimeout(timer);
  }, [joined]);
  return leaving;
}

function RunningSession({ app, view, instance, sessionId, done }: { app: App; view: View; instance: string; sessionId: string; done: boolean }) {
  const tr = useT();
  const picked = useSelection(view);
  const params = useMemo(() => ({ instance }), [instance]);
  const sessions = useTopic<{ sessions: SessionRowView[] }>(view, "harnix.instances.sessions", params);
  const taskParams = useMemo(() => ({ instance, session_id: sessionId }), [instance, sessionId]);
  const listed = useTopic<{ tasks: Task[] }>(view, "harnix.sessions.tasks", taskParams);
  const row = sessions?.sessions?.find((r) => r.session_id === sessionId);
  const live = (listed?.tasks ?? []).filter((task) => LIVE.has(task.status)).map((task) => task.task_id);
  const leaving = useLeaving(live);
  const tasks = (listed?.tasks ?? []).filter((task) => live.includes(task.task_id) || leaving.includes(task.task_id));
  const select = (kind: "session" | "task", taskId?: string) =>
    view.select(kind === "task" ? { kind, instance, sessionId, taskId: taskId! } : { kind, instance, sessionId });
  return (
    <>
      <div
        className={`tr l1${done ? " done" : ""}${isSelected(picked, { kind: "session", instance, sessionId }) ? " on" : ""}`}
        data-sid={sessionId}
        onClick={() => select("session")}
      >
        <span className="dot running" />
        <span className="nm">{row?.name || tr("sidebar.untitled")}</span>
        <span className="kd">{tr("system.kind.session")}</span>
        <span className="act">{row?.space ?? "sessions"}</span>
      </div>
      {tasks.map((task) => (
        <div
          key={task.task_id}
          className={`tr l2${live.includes(task.task_id) ? "" : " done"}${isSelected(picked, { kind: "task", instance, sessionId, taskId: task.task_id }) ? " on" : ""}`}
          data-task={task.task_id}
          data-status={task.status}
          onClick={() => select("task", task.task_id)}
        >
          <span className={`dot ${task.status}`} />
          <span className="nm">{tr(`system.task.${task.kind}`)}</span>
          <span className="act">{taskLabel(task)}</span>
          <span className="el">{duration(task)}</span>
        </div>
      ))}
    </>
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

function SessionOverview({ app, view, session }: { app: App; view: View; session: SessionKey }) {
  const tr = useT();
  const params = useMemo(() => ({ instance: session.instance }), [session.instance]);
  const listed = useTopic<{ sessions: SessionRowView[] }>(view, "harnix.instances.sessions", params);
  const row = listed?.sessions?.find((r) => r.session_id === session.sessionId);
  const space = row?.space ?? "sessions";
  return (
    <div className="system-pane" data-testid="system-session">
      <Facts
        rows={[
          [tr("system.fact.instance"), session.instance],
          [tr("system.fact.session"), session.sessionId],
          [tr("system.fact.name"), row?.name ?? ""],
          [tr("system.fact.space"), space],
          [tr("system.fact.scene"), row?.scene ?? ""],
          [tr("system.fact.project"), row?.project_root ?? ""],
        ]}
      />
      {space !== SYSTEM ? (
        <button
          type="button"
          className="btn sm outline wide"
          data-testid="open-in-space"
          onClick={() => {
            // The session space is listed by time and by project: open it where it belongs.
            const target = app.registry.at("space").find((c) => (c.sessions ?? c.id) === space && c.id !== SYSTEM)?.id ?? space;
            app.open(session, target);
            void app.switchSpace(target);
          }}
        >
          {tr("system.openInSpace", { space })}
        </button>
      ) : null}
      <div className="label">{tr("system.ops")}</div>
      <Operations app={app} view={view} session={session} name={row?.name ?? null} />
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
  duration_ms?: number | null;
}

type Filter = "all" | "failed" | "tools";

function failed(call: TraceCall): boolean {
  return call.outcome?.status === "error" || call.tools.some((tool) => tool.is_error);
}

function CallsTab({ app, session }: { app: App; session: SessionKey }) {
  const tr = useT();
  const [calls, setCalls] = useState<TraceCall[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
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
  if (error) return <div className="empty">{error}</div>;
  if (!calls) return null;
  if (open !== null) {
    const previous = calls.filter((c) => c.index < open && !c.purpose).pop() ?? null;
    return <CallDetail app={app} session={session} call={calls.find((c) => c.index === open)!} previous={previous} onBack={() => setOpen(null)} />;
  }
  if (!calls.length) return <div className="empty">{tr("system.noCalls")}</div>;
  const shown = calls.filter((call) => (filter === "failed" ? failed(call) : filter === "tools" ? call.tools.length > 0 : true));
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
      {turns.map((turn) => (
        <div key={turn}>
          <div className="turnh">{tr("system.turn", { turn })}</div>
          {shown
            .filter((c) => c.turn === turn)
            .map((call) => (
              <div key={call.index}>
                <button
                  type="button"
                  className={`step k-model${failed(call) && call.outcome?.status === "error" ? " bad" : ""}`}
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
                      {call.model} · {tokens(call.usage)} · {call.duration_ms != null ? `${(call.duration_ms / 1000).toFixed(1)}s` : "—"} · {call.stop_reason ?? call.outcome?.status}
                    </span>
                  </div>
                  <div className="l2">
                    <span className="in">{tr("system.callIn", { blocks: call.input.system.length, tools: call.input.tools, messages: call.input.messages })}</span>
                    <span className="arrow">→</span>
                    <span className="out">{callOut(call, tr)}</span>
                  </div>
                </button>
                {call.tools.map((tool) => (
                  <div key={tool.id} className={`step k-tool nested${tool.is_error ? " bad" : ""}`} data-tool={tool.id}>
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
}

interface HistoryMessage {
  seq: number;
  role: string;
  origin?: string | null;
  content: unknown;
}

function ContextTab({ app, session }: { app: App; session: SessionKey }) {
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
  const budget = Math.max(context.compact_threshold ?? 0, context.total_tokens ?? 0, 1);
  const pct = (n: number) => `${Math.min(100, (n / budget) * 100)}%`;
  return (
    <div className="system-pane" data-testid="system-context">
      <div className="note">{tr("system.contextNote")}</div>
      <div className="stack" data-testid="context-stack" title={tr("system.tokens", { count: context.total_tokens ?? 0 })}>
        <i className="sys" style={{ width: pct(systemTokens) }} />
        <i className="tools" style={{ width: pct(toolTokens) }} />
        <i className="msgs" style={{ width: pct(messageTokens) }} />
      </div>
      <div className="legend2">
        <span className="sys">{tr("system.legend.system", { count: systemTokens })}</span>
        <span className="tools">{tr("system.legend.tools", { count: toolTokens })}</span>
        <span className="msgs">{tr("system.legend.messages", { count: messageTokens })}</span>
      </div>
      <Facts
        rows={[
          [tr("system.fact.model"), context.model ?? ""],
          [tr("system.fact.total"), tr("system.tokens", { count: context.total_tokens ?? 0 })],
          [tr("system.fact.threshold"), context.compact_threshold ? tr("system.tokens", { count: context.compact_threshold }) : ""],
          [tr("system.fact.messages"), tr("system.messagesTokens", { count: context.messages?.count ?? 0, tokens: context.messages?.tokens ?? 0 })],
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
          tail={tr("system.tokens", { count: block.tokens })}
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
  const listed = useTopic<{ tasks: Task[] }>(view, "harnix.sessions.tasks", params);
  const tasks = listed?.tasks ?? [];
  if (!tasks.length) return <div className="note">{tr("system.noTasks")}</div>;
  const chosen = picked?.kind === "task" ? picked.taskId : null;
  return (
    <div className="system-pane tasks" data-testid="system-tasks">
      {tasks.map((task) => (
        <div key={task.task_id} className={`task${task.task_id === chosen ? " on" : ""}`} data-kind={task.kind} data-status={task.status}>
          <div className="task-h">
            <span className={`dot ${task.status}`} />
            <span className="kind">{tr(`system.task.${task.kind}`)}</span>
            <span className="what truncate" title={taskLabel(task)}>
              {taskLabel(task)}
            </span>
            <span className="when">{duration(task)}</span>
            {LIVE.has(task.status) ? (
              <button
                type="button"
                className="btn sm outline"
                data-testid="task-stop"
                onClick={() => void act(app, () => app.call("harnix.system.tasks.stop", { ...params, task_id: task.task_id }))}
              >
                {tr("system.stop")}
              </button>
            ) : null}
          </div>
          {task.task_id === chosen ? (
            <>
              {task.error ? <div className="hint err">{task.error}</div> : null}
              <pre className="hx-pre">{JSON.stringify(task.detail ?? {}, null, 2)}</pre>
            </>
          ) : null}
        </div>
      ))}
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
