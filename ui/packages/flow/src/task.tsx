/**
 * A task — a run of a flow in a project (`docs/glossary.md`, 项目任务) — as the projects space shows
 * it. The projects space defines `harnix-app.project.items` and knows no item kind by name; this
 * package registers the kind `flow.task` there: the project's runs from the `runs` topic as rows
 * (flow, state, "station k of n", the branch of one run in a worktree, a clock for one a schedule
 * started), "新任务" in the project's
 * "+" menu (`NewTask.tsx`), and a row opened as its main session in the middle with the 流程 tab
 * (`TaskTab.tsx`) in the right column.
 *
 * Where the main session opens: it belongs to the flow's backend space, and a call claiming the
 * projects space's would be refused (concept §5.4, `SPACE_MISMATCH`), so the task is entered the
 * way the system console enters a run — the flow's interface space brought to the front on it
 * (`FlowSpaceState.enter`), remembering where it came from for the way back.
 */

import { useEffect } from "react";
import { t, useT, type HostApi, type HostSessionRef as SessionRef, type Registry } from "@harnix/react";

import { useDefs, useRunView } from "./hooks";
import { isDryRun, mainSessionOf, methodName, phaseOf, runLive, runsOf, runsTopic, sameRoot } from "./model";
import { newTaskDialog } from "./NewTask";
import { PhaseBadge, taskStep } from "./RunPanel";
import type { FlowSpace } from "./space";
import type { RunSummary, RunView } from "./types";

/** The projects space's point for what a project holds besides its conversations. */
export const PROJECT_ITEMS = "harnix-app.project.items";
/** This package's kind there. */
export const TASK_KIND = "flow.task";
/** The right column's page type for a task: `openTab("flow", { params: { run } })`. */
export const TASK_TAB_KIND = "flow";

/** A row of the projects space's item kinds, as `harnix-app.project.items` takes it. */
export interface ItemRow {
  id: string;
  title: string;
  at?: number | string | null;
  running?: boolean;
  session?: SessionRef | null;
  data?: unknown;
}

/** What the projects space hands a kind about the project. */
export interface ItemContext {
  root: string;
}

/** A run's last activity, for ordering: when it finished, else when it started. */
export function activityOf(r: RunSummary): number | null {
  const at = Number(r.finished_at ?? r.created_at ?? NaN);
  return Number.isFinite(at) ? at : null;
}

/** The tasks of a project among the runs: those of its root, dry runs left out; newest activity first. */
export function taskRows(runs: RunSummary[], root: string, sessions: ReadonlyMap<string, SessionRef> = new Map()): ItemRow[] {
  return runs
    .filter((r) => sameRoot(r.project_root, root) && !r.dry && !isDryRun(r.id))
    .map((r) => ({ id: r.id, title: r.title || r.subject || r.id, at: activityOf(r), running: runLive(r), session: sessions.get(r.id) ?? null, data: r }))
    .sort((a, b) => (b.at ?? -1) - (a.at ?? -1) || b.id.localeCompare(a.id));
}

/**
 * The runs as the `runs` topic last said them, followed while someone listens — the projects
 * tree reads a project's rows on each draw, and this is what it reads from. The main sessions
 * the rows' bodies come to know are kept too, so a row is marked current while its session is open.
 */
export class ProjectTasks {
  private runs: RunSummary[] = [];
  private readonly sessions = new Map<string, SessionRef>();
  private readonly listeners = new Set<() => void>();
  private off: (() => void) | null = null;

  constructor(
    private readonly host: HostApi,
    readonly module: string,
  ) {}

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    if (!this.off) {
      this.off = this.host.subscribe(runsTopic(this.module), {}, (value) => {
        this.runs = runsOf(value);
        this.changed();
      });
    }
    return () => {
      this.listeners.delete(fn);
      if (!this.listeners.size) {
        this.off?.();
        this.off = null;
      }
    };
  };

  all(): RunSummary[] {
    return this.runs;
  }

  rows(root: string): ItemRow[] {
    return taskRows(this.runs, root, this.sessions);
  }

  /** A row's body learnt the run's main session: rows say it from now on. */
  learned(run: string, session: SessionRef | null): void {
    if (!session) return;
    const known = this.sessions.get(run);
    if (known && known.instance === session.instance && known.sessionId === session.sessionId) return;
    this.sessions.set(run, session);
    this.changed();
  }

  private changed(): void {
    for (const fn of [...this.listeners]) fn();
  }
}

/* ── the row ──────────────────────────────────────────────────────────── */

/** A task on its row: the flow's title, its state, where it is, and a clock when a schedule started it. */
export function TaskRow({ run, module, tasks }: { run: RunSummary; module: string; tasks: ProjectTasks }) {
  const t = useT();
  const defs = useDefs(module);
  const view = useRunView(module, run.id);
  const main = mainSessionOf(view?.record);
  useEffect(() => tasks.learned(run.id, main), [tasks, run.id, main?.instance, main?.sessionId]);
  const phase = phaseOf(view ? { phase: view.phase, status: view.record.status } : run);
  const step = taskStep(view);
  const flow = defs.list.find((d) => d.name === run.flow)?.title || run.flow;
  const scheduled = run.trigger?.kind === "schedule";
  const branch = run.worktree?.branch ?? null;
  return (
    <>
      {runLive(run) ? <span className="live" /> : null}
      <span className="tt name hx-flow-task-title" title={`${run.title || run.subject || run.id} · ${flow}`}>
        {run.title || run.subject || run.id}
      </span>
      <span className="meta hx-flow-task-meta" data-testid="task-meta" data-phase={phase.state} data-step={step ? `${step.k}/${step.n}` : undefined}>
        <span className="hx-flow-task-flow">{flow}</span>
        <PhaseBadge phase={phase} />
        {step ? <span className="hx-flow-task-step">{t("flowui.task.step", { k: step.k, n: step.n })}</span> : null}
        {branch ? (
          <span className="hx-flow-task-branch" data-testid="task-branch" data-outcome={run.worktree?.outcome?.kind ?? undefined} title={t("flowui.task.inWorktree", { branch })}>
            {branch}
          </span>
        ) : null}
        {/* Badges about time: the clock of one a schedule started; a schedule's next firing goes here too. */}
        <span className="hx-flow-task-badges" data-testid="task-badges">
          {scheduled ? (
            <svg className="glyph hx-flow-task-clock" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-label={t("flowui.task.scheduled")} role="img">
              <circle cx="12" cy="12" r="8.5" />
              <path d="M12 7.5V12l3 2" />
            </svg>
          ) : null}
        </span>
      </span>
    </>
  );
}

/* ── opening a task ───────────────────────────────────────────────────── */

/** `flow.get {run}` until the run has its main session, or `tries` reads have passed. */
export async function awaitMain(host: HostApi, module: string, run: string, tries = 30, every = 500): Promise<SessionRef | null> {
  for (let i = 0; i < tries; i++) {
    try {
      const main = mainSessionOf((await host.call<RunView>(methodName(module, "get"), { run }))?.record);
      if (main) return main;
    } catch {
      return null;
    }
    await new Promise((resolve) => setTimeout(resolve, every));
  }
  return null;
}

/**
 * Open a task: the flow's interface space brought to the front with the run's main session in the
 * middle, the panel beside the conversation closed (the right column's 流程 tab shows the run),
 * and that tab opened on it. A run just started has its main session a moment later: waited for.
 */
export async function openTask(host: HostApi, r: Registry, space: FlowSpace, run: string, flow: string): Promise<void> {
  const here = host.spaces.current();
  const from = r.at("space").find((c) => c.id === here);
  let title = here;
  try {
    title = from?.title() ?? here;
  } catch {
    // A space whose title fails is named by its id.
  }
  await space.state.enter(flow, run, { from: from ? { space: from.id, title } : undefined });
  space.state.closeDetail();
  const main = await awaitMain(host, space.module, run);
  if (!main) return;
  const open = host.session.current();
  if (!open || open.sessionId !== main.sessionId || open.instance !== main.instance) {
    try {
      await host.session.open(main);
    } catch {
      return;
    }
  }
  try {
    host.sidebar.openTab(TASK_TAB_KIND, { params: { run } });
  } catch {
    // No session open after all (gone between the answer and now): the middle says so.
  }
}

/* ── the registration ─────────────────────────────────────────────────── */

/**
 * The kind `flow.task` of `harnix-app.project.items`: registered when the projects space defines
 * the point, dropped quietly when nothing does (a product without the Harnix app). `space` answers
 * the flow space once it is registered — after this, when the flows are a space of their own.
 */
export function registerTaskItems(r: Registry, host: HostApi, module: string, space: () => FlowSpace | null, owner: string): ProjectTasks {
  const tasks = new ProjectTasks(host, module);
  const flowOf = (row: ItemRow) => (row.data as RunSummary | undefined)?.flow ?? "";
  r.register(
    PROJECT_ITEMS,
    {
      id: TASK_KIND,
      title: () => t("flowui.task.kind"),
      rows: (root: string) => tasks.rows(root),
      subscribe: tasks.subscribe,
      render: (row: ItemRow) => <TaskRow run={row.data as RunSummary} module={module} tasks={tasks} />,
      open: async (row: ItemRow) => {
        const view = space();
        if (view) await openTask(host, r, view, row.id, flowOf(row));
      },
      create: {
        label: () => t("flowui.task.new"),
        icon: "fork",
        run: (context: ItemContext) =>
          newTaskDialog(host, module, { root: context.root, runs: () => tasks.all() }, async (run, flow) => {
            const view = space();
            if (view) await openTask(host, r, view, run, flow);
          }),
      },
    },
    owner,
    { orElse: () => undefined },
  );
  return tasks;
}
