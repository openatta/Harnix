/**
 * The system space (design §9): everything this Harnix runs, four layers deep, and the hands to act
 * on it. The left column is the instance tree — each instance, and under the chosen one its
 * sessions, of whatever space; the centre is the chosen session's conversation, the same panel every
 * space uses; the right column inspects it: the session and its tasks, each model call's envelope
 * and what changed from the one before, the context the next call will carry, and the instance.
 *
 * Registered only when the backend offers the system methods (`[system_space] enabled`), and built
 * on the public calls alone: `harnix.system.*` for what the engine keeps behind the system module,
 * `session.*` and `harnix.instances.*` claiming the system space for the rest — every one of them
 * audited. What Agix does not offer has no button here (§9.3).
 */

import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { errorText } from "@harnix/core";
import { Disclosure, clip, t, useT } from "@harnix/react";

import { keyOf, type App, type SessionKey, type ShellState } from "./app.ts";
import { confirmDialog, renameDialog } from "./dialogs.tsx";
import { change, sizeSummary, type Header } from "./envelope.tsx";
import { Glyph } from "./glyph.tsx";

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

export function registerSystem(app: App): void {
  if (!app.snapshot.reachable.includes("harnix.system.context")) return;
  const view = new View(app);
  app.registry.register(
    "space",
    {
      id: SYSTEM,
      icon: "terminal",
      order: 90,
      title: () => t("space.system"),
      subtitle: () => t("space.system.subtitle"),
      center: "chat",
      sidebar: () => <InstanceTree app={app} view={view} />,
      tabs: [
        { id: "system.session", title: () => t("system.tab.session"), render: () => <SessionTab app={app} view={view} /> },
        { id: "system.calls", title: () => t("system.tab.calls"), render: () => <CallsTab app={app} /> },
        { id: "system.context", title: () => t("system.tab.context"), render: () => <ContextTab app={app} /> },
        { id: "system.instance", title: () => t("system.tab.instance"), render: () => <InstanceTab app={app} view={view} /> },
      ],
    },
    OWNER,
  );
}

/* ── which instance is open, and the topics that keep the tree current ─── */

/**
 * The chosen instance, and the hub topics this space watches. A topic is subscribed once however
 * many parts of the page read it, and given up when the last one stops.
 */
class View {
  private chosen: string | null = null;
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

  instance(): string {
    return this.chosen ?? this.app.snapshot.currentBySpace[SYSTEM]?.instance ?? "system";
  }

  choose(instance: string): void {
    this.chosen = instance;
    for (const fn of [...this.listeners]) fn();
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

function useView(view: View): string {
  return useSyncExternalStore(view.subscribe, () => view.instance());
}

function useTopic<T>(view: View, topic: string, params?: Record<string, unknown>): T | undefined {
  const key = topicKey(topic, params);
  const [, bump] = useState(0);
  useEffect(() => view.watch(topic, params, () => bump((n) => n + 1)), [view, key]);
  return view.value(topic, params) as T | undefined;
}

function useShell(app: App): ShellState {
  return useSyncExternalStore(app.subscribe, app.getSnapshot);
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

/* ── the left column: the instance tree ───────────────────────────────── */

function InstanceTree({ app, view }: { app: App; view: View }) {
  const tr = useT();
  const s = useShell(app);
  const chosen = useView(view);
  const listed = useTopic<{ instances: InstanceRow[] }>(view, "harnix.instances");
  // The inspector is this space's purpose: it opens with the space.
  useEffect(() => app.toggleDetails(true), [app]);
  const instances = listed?.instances ?? [];
  return (
    <div className="system-tree" data-testid="system-tree">
      {instances.map((instance) => {
        const open = instance.id === chosen;
        return (
          <div key={instance.id} className="inst-group">
            <button
              type="button"
              className={`srow inst${open ? " on" : ""}`}
              data-instance={instance.id}
              data-state={instance.status.state}
              title={instance.status.reason ?? instance.status.state}
              onClick={() => view.choose(instance.id)}
            >
              <span className={`dot ${instance.status.state}`} />
              <span className="tt name">{instance.id}</span>
              <span className="meta carrier">{tr(`system.carrier.${instance.carrier}`)}</span>
            </button>
            {open ? <InstanceSessions app={app} view={view} instance={instance.id} s={s} /> : null}
          </div>
        );
      })}
    </div>
  );
}

function InstanceSessions({ app, view, instance, s }: { app: App; view: View; instance: string; s: ShellState }) {
  const tr = useT();
  const params = useMemo(() => ({ instance }), [instance]);
  const listed = useTopic<{ sessions: SessionRowView[] }>(view, "harnix.instances.sessions", params);
  if (!listed) return null;
  const rows = [...(listed.sessions ?? [])].sort((a, b) => String(b.last_active ?? "").localeCompare(String(a.last_active ?? "")));
  if (!rows.length) return <div className="empty inst-empty">{tr("sidebar.noSessions")}</div>;
  return (
    <div className="inst-sessions" data-instance={instance}>
      {rows.map((row) => {
        const on = s.current?.instance === instance && s.current.sessionId === row.session_id;
        const running = s.running[keyOf(instance, row.session_id)];
        return (
          <button
            type="button"
            key={row.session_id}
            className={`srow sub${on ? " on" : ""}`}
            data-sid={row.session_id}
            data-instance={instance}
            title={[row.name, row.scene, row.project_root].filter(Boolean).join(" · ")}
            onClick={() => app.open({ instance, sessionId: row.session_id }, SYSTEM)}
          >
            {running ? <span className="live" /> : null}
            <span className="tt name">{row.name || tr("sidebar.untitled")}</span>
            <span className="space-tag" data-space={row.space}>
              {row.space ?? "sessions"}
            </span>
          </button>
        );
      })}
    </div>
  );
}

/* ── the session tab: facts, operations, tasks ────────────────────────── */

function SessionTab({ app, view }: { app: App; view: View }) {
  const tr = useT();
  const s = useShell(app);
  const key = s.currentBySpace[SYSTEM];
  const params = useMemo(() => (key ? { instance: key.instance } : undefined), [key?.instance]);
  const listed = useTopic<{ sessions: SessionRowView[] }>(view, "harnix.instances.sessions", params ?? { instance: view.instance() });
  if (!key) return <div className="empty">{tr("system.pickSession")}</div>;
  const row = listed?.sessions?.find((r) => r.session_id === key.sessionId);
  const space = row?.space ?? "sessions";
  return (
    <div className="system-pane" data-testid="system-session">
      <Facts
        rows={[
          [tr("system.fact.instance"), key.instance],
          [tr("system.fact.session"), key.sessionId],
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
            app.open(key, space);
            void app.switchSpace(space);
          }}
        >
          {tr("system.openInSpace", { space })}
        </button>
      ) : null}
      <div className="label">{tr("system.ops")}</div>
      <Operations app={app} session={key} name={row?.name ?? null} />
      <div className="label">{tr("system.tasks")}</div>
      <Tasks app={app} view={view} session={key} />
    </div>
  );
}

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

function Operations({ app, session, name }: { app: App; session: SessionKey; name: string | null }) {
  const tr = useT();
  const target = { instance: session.instance, session_id: session.sessionId };
  const system = (method: string, extra: Record<string, unknown> = {}) => app.call(`harnix.system.${method}`, { ...target, ...extra });
  const settled = () => void app.refreshSessions();
  const ops: { id: string; icon: string; label: string; run: () => void; danger?: boolean }[] = [
    {
      id: "compact",
      icon: "archive",
      label: tr("system.op.compact"),
      run: () => void act(app, () => system("compact")).then(settled),
    },
    {
      id: "clear",
      icon: "refresh",
      label: tr("system.op.clear"),
      run: () => confirmDialog(app, tr("system.confirmClear"), async () => {
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
          const forked = await app.sessionCall<{ session_id: string; instance?: string }>("session.fork", target);
          app.open({ instance: forked.instance ?? session.instance, sessionId: forked.session_id }, SYSTEM);
          settled();
        }),
    },
    {
      id: "model",
      icon: "agent",
      label: tr("system.op.model"),
      run: () => renameDialog(app, tr("system.op.model"), app.snapshot.hello?.engine?.model ?? "", (model) => system("model", { model: model.trim() })),
    },
    {
      id: "interrupt",
      icon: "stop",
      label: tr("system.op.interrupt"),
      run: () => void act(app, () => app.sessionCall("session.interrupt", target)),
    },
    {
      id: "rename",
      icon: "edit",
      label: tr("system.op.rename"),
      run: () => renameDialog(app, tr("system.op.rename"), name, (value) => system("rename", { name: value.trim() })),
    },
    {
      id: "close",
      icon: "close",
      label: tr("system.op.close"),
      run: () => void act(app, () => app.sessionCall("session.close", target)).then(settled),
    },
    {
      id: "delete",
      icon: "trash",
      label: tr("system.op.delete"),
      danger: true,
      run: () =>
        confirmDialog(app, tr("system.confirmDelete"), async () => {
          await app.sessionCall("session.delete", target);
          app.closeModal();
          app.forget(session);
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

function Tasks({ app, view, session }: { app: App; view: View; session: SessionKey }) {
  const tr = useT();
  const params = useMemo(() => ({ instance: session.instance, session_id: session.sessionId }), [session.instance, session.sessionId]);
  const listed = useTopic<{ tasks: Task[] }>(view, "harnix.sessions.tasks", params);
  const tasks = listed?.tasks ?? [];
  if (!tasks.length) return <div className="note">{tr("system.noTasks")}</div>;
  return (
    <div className="tasks" data-testid="system-tasks">
      {tasks.map((task) => (
        <div key={task.task_id} className="task" data-kind={task.kind} data-status={task.status}>
          <span className={`dot ${task.status}`} />
          <span className="kind">{tr(`system.task.${task.kind}`)}</span>
          <span className="what truncate" title={taskLabel(task)}>
            {taskLabel(task)}
          </span>
          <span className="when">{duration(task)}</span>
          {task.status === "running" || task.status === "waiting" ? (
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

/* ── the calls tab: each envelope, and what moved since the one before ─── */

function CallsTab({ app }: { app: App }) {
  const tr = useT();
  const s = useShell(app);
  const key = s.currentBySpace[SYSTEM];
  const [headers, setHeaders] = useState<Header[] | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  useEffect(() => {
    if (!key) return;
    let alive = true;
    const load = () =>
      app
        .call<{ headers?: Header[] }>("harnix.system.calls", { instance: key.instance, session_id: key.sessionId })
        .then((r) => alive && (setHeaders(r.headers ?? []), setFailed(null)))
        .catch((e) => alive && setFailed(errorText(e)));
    void load();
    const off = app.client.onNotify((method, params) => {
      if (method === "harnix.turn_settled" && params.session_id === key.sessionId) void load();
    });
    return () => {
      alive = false;
      off();
    };
  }, [app, key?.instance, key?.sessionId]);
  if (!key) return <div className="empty">{tr("system.pickSession")}</div>;
  if (failed) return <div className="empty">{failed}</div>;
  if (!headers) return null;
  if (!headers.length) return <div className="empty">{tr("system.noCalls")}</div>;
  return (
    <div className="system-pane calls" data-testid="system-calls">
      <div className="note">{tr("system.callsNote")}</div>
      {headers.map((header, i) => {
        const previous = i ? headers[i - 1]! : null;
        const moved = change(previous, header);
        return (
          <Disclosure
            key={i}
            testId="system-call"
            state="ok"
            summary={
              <>
                <span className="hx-tool-name">#{i + 1}</span>
                <span className="hx-tool-arg">
                  {tr("request.turnStep", { turn: header.turn ?? "?", step: header.step ?? "?" })} · {header.model ?? "?"}
                </span>
              </>
            }
            tail={tr(`request.change.${moved}`)}
          >
            <div className="note">{sizeSummary(header)}</div>
            <Diff previous={previous} header={header} />
            <div className="actions">
              <button
                type="button"
                className="btn sm outline"
                data-testid="open-in-details"
                onClick={() => app.showDetail({ kind: "request", name: tr("request.title"), header, previous, change: moved })}
              >
                {tr("flow.openInDetails")}
              </button>
            </div>
          </Disclosure>
        );
      })}
    </div>
  );
}

/** What differs between two envelopes, block by block and tool by tool. */
function Diff({ previous, header }: { previous: Header | null; header: Header }) {
  const tr = useT();
  if (!previous) return <div className="diff none">{tr("system.diff.first")}</div>;
  const lines: { sign: "+" | "-" | "~"; text: string }[] = [];
  const before = previous.system ?? [];
  const after = header.system ?? [];
  for (let i = 0; i < Math.max(before.length, after.length); i++) {
    const a = before[i];
    const b = after[i];
    const label = (b ?? a)?.source ?? `#${i + 1}`;
    if (!a) lines.push({ sign: "+", text: tr("system.diff.block", { label, chars: (b?.text ?? "").length }) });
    else if (!b) lines.push({ sign: "-", text: tr("system.diff.block", { label, chars: (a.text ?? "").length }) });
    else if (a.text !== b.text || a.cache !== b.cache)
      lines.push({ sign: "~", text: tr("system.diff.changed", { label, from: (a.text ?? "").length, to: (b.text ?? "").length }) });
  }
  const names = (h: Header) => new Set((h.tools ?? []).map((tool) => tool.name));
  const had = names(previous);
  const has = names(header);
  for (const name of has) if (!had.has(name)) lines.push({ sign: "+", text: tr("system.diff.tool", { name }) });
  for (const name of had) if (!has.has(name)) lines.push({ sign: "-", text: tr("system.diff.tool", { name }) });
  if (previous.model !== header.model) lines.push({ sign: "~", text: `${previous.model ?? "?"} → ${header.model ?? "?"}` });
  if (previous.max_tokens !== header.max_tokens) lines.push({ sign: "~", text: `max_tokens ${previous.max_tokens ?? "?"} → ${header.max_tokens ?? "?"}` });
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

function ContextTab({ app }: { app: App }) {
  const tr = useT();
  const s = useShell(app);
  const key = s.currentBySpace[SYSTEM];
  const [context, setContext] = useState<Context | null>(null);
  const [history, setHistory] = useState<{ messages: HistoryMessage[]; total: number } | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [round, setRound] = useState(0);
  useEffect(() => {
    if (!key) return;
    let alive = true;
    const target = { instance: key.instance, session_id: key.sessionId };
    const load = async () => {
      try {
        const [ctx, first] = await Promise.all([
          app.call<Context>("harnix.system.context", target),
          app.sessionCall<{ total: number }>("session.history", target, { offset: 0, limit: 1 }),
        ]);
        const offset = Math.max(0, first.total - 50);
        const page = await app.sessionCall<{ messages: HistoryMessage[]; total: number }>("session.history", target, { offset, limit: 50 });
        if (!alive) return;
        setContext(ctx);
        setHistory({ messages: page.messages ?? [], total: page.total });
        setFailed(null);
      } catch (e) {
        if (alive) setFailed(errorText(e));
      }
    };
    void load();
    const off = app.client.onNotify((method, params) => {
      if (method === "harnix.turn_settled" && params.session_id === key.sessionId) void load();
    });
    return () => {
      alive = false;
      off();
    };
  }, [app, key?.instance, key?.sessionId, round]);
  if (!key) return <div className="empty">{tr("system.pickSession")}</div>;
  if (failed) return <div className="empty" data-testid="system-context-failed">{failed}</div>;
  if (!context) return null;
  const target = { instance: key.instance, session_id: key.sessionId };
  const total = history?.total ?? 0;
  const again = () => setRound((n) => n + 1);
  return (
    <div className="system-pane" data-testid="system-context">
      <div className="note">{tr("system.contextNote")}</div>
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

/* ── the instance tab ─────────────────────────────────────────────────── */

interface Template {
  id: string;
  carrier: string;
  params: string[];
}

function InstanceTab({ app, view }: { app: App; view: View }) {
  const tr = useT();
  const chosen = useView(view);
  const listed = useTopic<{ instances: InstanceRow[] }>(view, "harnix.instances");
  const instance = listed?.instances.find((i) => i.id === chosen);
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
                      view.choose("system");
                    })
                  }
                >
                  <Glyph name="stop" className="glyph" />
                  {tr("system.stop")}
                </button>
              ) : null}
            </div>
          ) : null}
        </>
      ) : (
        <div className="empty">{tr("system.pickInstance")}</div>
      )}
      <div className="label">{tr("system.newInstance")}</div>
      <NewInstance app={app} view={view} />
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
      view.choose(built.instance?.id ?? built.id ?? name.trim());
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
