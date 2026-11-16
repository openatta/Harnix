/**
 * A task in a project: the rows the projects tree gets (the project's runs, dry runs left out,
 * by activity), the kind registered where the projects space defines its point and the 流程 tab
 * type, the new-task dialog's form from an `inputs` schema and what it sends to `start`, a task
 * opened (its main session in the space in front where the module shows it there, else the flow
 * space entered on it; the tab), and the tab drawing the run.
 */

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { HostProvider, Registry, TabInfoProvider, setLocale, type HostApi, type HostSessionRef, type TabInfo } from "@harnix/react";

import { collectInputs, defOptions, defaultOutputDir, fieldsOf, initialText, newTaskDialog, noWorktreeText, workspaceParam } from "../src/NewTask";
import { register } from "../src/register";
import { WorktreeLine, currentStation, taskStep, worktreeOpen } from "../src/RunPanel";
import { registerSystemFlows } from "../src/space";
import { registerFlowStrings } from "../src/strings";
import { INBOX, PROJECT_ITEMS, PROJECT_PANELS, ProjectTasks, TASK_KIND, TASK_TAB_KIND, TaskRow, TasksPanel, enterTask, openTask, taskCounts, taskRows, waitingItems } from "../src/task";
import { START_ACTION, TASK_BADGE, type StartAction, type StartDraft, type TaskBadge } from "../src/points";
import { TaskTab } from "../src/TaskTab";
import type { DefSummary, FlowDef, RunSummary, RunView, WorktreeRec } from "../src/types";

afterEach(() => {
  cleanup();
  localStorage.clear();
});
setLocale("en");
registerFlowStrings(new Registry());

const ROOT = "/work/alpha";
const DEF: FlowDef = {
  name: "example-review",
  title: "Example: review and approve",
  stations: [
    { id: "review", executor: { kind: "session" }, skill: "review", output: { type: "object" }, exit: [{ kind: "output" }] },
    { id: "approve", exit: [{ kind: "confirm" }], rework_to: ["review"] },
    { id: "sign", needs: ["approve"], executor: { kind: "external", who: "clerk" }, output: { type: "object" }, exit: [{ kind: "output" }] },
  ],
};
const DEFS: DefSummary[] = [
  { name: "example-review", title: "Example: review and approve", digest: "a", origin: "builtin", valid: true, inputs: { type: "object", properties: { depth: { type: "integer", default: 2 }, target: { type: "string", description: "What to look at." } }, required: ["target"] } },
  { name: "broken", title: "Broken", digest: "b", origin: "user", valid: false, inputs: { type: "object", properties: {}, required: [] } },
];

const WORKTREE: WorktreeRec = { path: `${ROOT}/.harnix/worktrees/example-review-1`, branch: "harnix/example-review-1", base: "0123abcd" };

const summary = (over: Partial<RunSummary>): RunSummary => ({
  id: "example-review-1",
  flow: "example-review",
  subject: "repo",
  status: "active",
  rev: 1,
  created_at: Date.parse("2026-03-02T10:00:00Z"),
  project_root: ROOT,
  workspace: "project",
  trigger: { kind: "manual", by: { kind: "device", id: "d" } },
  phase: { state: "running" },
  ...over,
});

const view = (over: Partial<RunView["record"]> = {}, phase: RunView["phase"] = { state: "waiting", reason: "confirm" }): RunView => ({
  record: {
    id: "example-review-1",
    flow: "example-review",
    def_digest: "a",
    def: DEF,
    subject: "repo",
    status: "active",
    rev: 3,
    seq: 3,
    created_at: Date.parse("2026-03-02T10:00:00Z"),
    created_by: { kind: "device", id: "d" },
    inputs: {},
    workspace: "project",
    project_root: ROOT,
    output_dir: "flows/example-review-2026030201",
    trigger: { kind: "manual", by: { kind: "device", id: "d" } },
    session_keys: { "@main": "main/main-1" },
    stations: {
      review: { status: "passed", round: 1, rounds_total: 1, auto_reworks: 0, entered_at: 1, left_at: 2, tasks: [{ id: "review", status: "submitted", attempt: 1, output: { summary: "fine" } }], checks: [{ kind: "output", label: "output", round: 1, status: "pass", at: 2 }] },
      approve: { status: "working", round: 1, rounds_total: 1, auto_reworks: 0, entered_at: 2, tasks: [] },
      sign: { status: "pending", round: 0, rounds_total: 0, auto_reworks: 0 },
    },
    ...over,
  },
  unmet: {},
  phase,
});

/** A host with topics that can be pushed to, a front space, a session, and the right column. */
function fakeHost(answers: Record<string, unknown> = {}, confirmed = true) {
  const calls: [string, unknown][] = [];
  const topics = new Map<string, Set<(v: unknown) => void>>();
  const values = new Map<string, unknown>();
  const changes = new Set<(s: HostSessionRef | null) => void>();
  const opened: HostSessionRef[] = [];
  const tabs: [string, unknown][] = [];
  const dialogs: ((close: (r: unknown) => void) => unknown)[] = [];
  const confirms: [string, unknown][] = [];
  let current: HostSessionRef | null = null;
  let front = "projects";
  const keyOf = (topic: string, params?: Record<string, unknown>) => `${topic} ${JSON.stringify(params ?? {})}`;
  const host = {
    call: vi.fn(async (method: string, params?: unknown) => {
      calls.push([method, params]);
      const answer = answers[method];
      return typeof answer === "function" ? (answer as (p: unknown) => unknown)(params) : (answer ?? null);
    }),
    subscribe: vi.fn((topic: string, params: Record<string, unknown> | undefined, fn: (v: unknown) => void) => {
      const key = keyOf(topic, params);
      const set = topics.get(key) ?? new Set();
      set.add(fn);
      topics.set(key, set);
      if (values.has(key)) fn(values.get(key));
      return () => set.delete(fn);
    }),
    session: {
      current: () => current,
      onChange: (fn: (s: HostSessionRef | null) => void) => {
        changes.add(fn);
        return () => changes.delete(fn);
      },
      open: vi.fn(async (ref: HostSessionRef) => {
        opened.push(ref);
        current = ref;
        for (const fn of [...changes]) fn(ref);
      }),
    },
    spaces: {
      current: () => front,
      switch: vi.fn(async (id: string) => {
        front = id;
        for (const fn of [...changes]) fn(current);
      }),
    },
    sidebar: { openTab: vi.fn((kind: string, options?: unknown) => (tabs.push([kind, options]), "tab-1")) },
    details: { open: vi.fn(), tab: () => "", onTab: () => () => undefined },
    dialogs: {
      open: vi.fn(async (render: (close: (r: unknown) => void) => unknown) => void dialogs.push(render)),
      confirm: vi.fn(async (text: string, opts?: unknown) => (confirms.push([text, opts]), confirmed)),
    },
    composer: { insert: vi.fn() },
    redraw: vi.fn(),
    t: (k: string) => k,
  } as unknown as HostApi;
  return {
    host,
    calls,
    opened,
    tabs,
    dialogs,
    confirms,
    push(topic: string, params: Record<string, unknown> | undefined, value: unknown) {
      const key = keyOf(topic, params);
      values.set(key, value);
      for (const fn of [...(topics.get(key) ?? [])]) fn(value);
    },
    front: (space: string) => (front = space),
  };
}

const settle = () => act(async () => {});
const tabInfo = (session: HostSessionRef | null, params: Record<string, unknown> = {}, revision = 1): TabInfo => ({
  tab: { id: "t1", kind: TASK_TAB_KIND, address: null, session, visible: true, navigation: { params, revision }, signal: new AbortController().signal, actions: { close: () => undefined, openResource: () => "t", openTab: () => "t", onClose: () => () => undefined } },
});

describe("the rows of a project", () => {
  it("are the project's runs, the trailing slash aside, dry runs left out, newest activity first", () => {
    const runs = [
      summary({ id: "example-review-1", created_at: 10 }),
      summary({ id: "example-review-2", created_at: 20, project_root: `${ROOT}/`, status: "finished", finished_at: 50, phase: { state: "finished" } }),
      summary({ id: "example-review-3", created_at: 30, project_root: "/work/beta" }),
      summary({ id: "dry-example-review-1", created_at: 40, dry: true }),
      summary({ id: "example-review-4", created_at: 5, project_root: null }),
    ];
    const rows = taskRows(runs, ROOT, new Map([["example-review-1", { instance: "main", sessionId: "main-1" }]]));
    expect(rows.map((r) => [r.id, r.at, r.running, r.session?.sessionId ?? null])).toEqual([
      ["example-review-2", 50, false, null],
      ["example-review-1", 10, true, "main-1"],
    ]);
    expect(rows[0]!.title).toBe("repo");
  });

  it("come from the runs topic while someone listens, and say a run's main session once a row learnt it", () => {
    const fake = fakeHost();
    const tasks = new ProjectTasks(fake.host, "flow");
    expect(tasks.rows(ROOT)).toEqual([]);
    const heard = vi.fn();
    const off = tasks.subscribe(heard);
    expect(fake.host.subscribe).toHaveBeenCalledWith("flow.runs", {}, expect.any(Function));
    fake.push("flow.runs", {}, { runs: [summary({})] });
    expect(heard).toHaveBeenCalledTimes(1);
    expect(tasks.rows(ROOT).map((r) => r.id)).toEqual(["example-review-1"]);
    tasks.learned("example-review-1", { instance: "main", sessionId: "main-1" });
    expect(heard).toHaveBeenCalledTimes(2);
    expect(tasks.rows(ROOT)[0]!.session).toEqual({ instance: "main", sessionId: "main-1" });
    off();
  });

  it("a row: the flow's title, the state, where it is, the clock of a scheduled one", async () => {
    const fake = fakeHost({ "flow.defs": DEFS });
    const tasks = new ProjectTasks(fake.host, "flow");
    const run = summary({ trigger: { kind: "schedule", schedule: "nightly", fire_at: 0 } });
    // The tree listens: the runs are followed.
    tasks.subscribe(() => undefined);
    fake.push("flow.runs", {}, { runs: [run] });
    expect(tasks.rows(ROOT)[0]?.session).toBeNull();
    render(
      <HostProvider host={fake.host}>
        <TaskRow run={run} module="flow" tasks={tasks} />
      </HostProvider>,
    );
    await settle();
    act(() => fake.push("flow.run", { run: "example-review-1" }, view()));
    const meta = screen.getByTestId("task-meta");
    expect(meta.dataset.phase).toBe("waiting");
    expect(meta.dataset.step).toBe("2/3");
    expect(meta.textContent).toContain("Example: review and approve");
    expect(meta.textContent).toContain("station 2/3");
    expect(screen.getByTestId("task-badges").querySelector("svg")).not.toBeNull();
    // The row learnt the run's main session from the run topic: the rows say it now.
    expect(tasks.rows(ROOT)[0]?.session).toEqual({ instance: "main", sessionId: "main-1" });
  });

  it("a row of a task run in a worktree says its branch, and what became of it", async () => {
    const fake = fakeHost({ "flow.defs": DEFS });
    const tasks = new ProjectTasks(fake.host, "flow");
    const run = summary({ workspace: "worktree", worktree: { ...WORKTREE, outcome: { kind: "merged", at: 1, by: { kind: "device", id: "d" }, into: "main" } } });
    render(
      <HostProvider host={fake.host}>
        <TaskRow run={run} module="flow" tasks={tasks} />
      </HostProvider>,
    );
    await settle();
    const branch = screen.getByTestId("task-branch");
    expect(branch.textContent).toBe("harnix/example-review-1");
    expect(branch.dataset.outcome).toBe("merged");
    expect(branch.title).toContain("harnix/example-review-1");
  });
});

describe("where a run is", () => {
  it("is the first station being worked on, held or unmet; the last once over; what has been entered before any", () => {
    expect(currentStation(view())).toBe("approve");
    expect(taskStep(view())).toEqual({ k: 2, n: 3 });
    const over = view({ status: "finished", stations: { review: { status: "passed", round: 1, rounds_total: 1, auto_reworks: 0 }, approve: { status: "passed", round: 1, rounds_total: 1, auto_reworks: 0 }, sign: { status: "passed", round: 1, rounds_total: 1, auto_reworks: 0 } } }, { state: "finished" });
    expect(taskStep(over)).toEqual({ k: 3, n: 3 });
    const fresh = view({ stations: { review: { status: "pending", round: 0, rounds_total: 0, auto_reworks: 0 } } }, { state: "running" });
    expect(taskStep(fresh)).toEqual({ k: 0, n: 3 });
    expect(taskStep(null)).toBeNull();
  });
});

describe("the registration", () => {
  const defined = (fake: ReturnType<typeof fakeHost>) => {
    const r = new Registry();
    r.activate("system", () => r.definePoint("system.console.tab", { kind: "register" }));
    r.activate("harnix-app", () => r.definePoint(PROJECT_ITEMS, { kind: "register" }));
    r.activate("flow", () => register(r, fake.host));
    r.settle();
    return r;
  };

  it("the kind flow.task where the projects space defines its point, with 新任务; the 流程 tab type, kept mounted", () => {
    const r = defined(fakeHost());
    const kind = r.list<{ id: string; title(): string; create?: { label(): string } }>(PROJECT_ITEMS)[0]!;
    expect([kind.id, kind.owner, kind.title(), kind.create?.label()]).toEqual([TASK_KIND, "flow", "Tasks", "New task"]);
    const tab = r.at("sidebar.tab").find((x) => x.kind === TASK_TAB_KIND)!;
    expect([tab.id, tab.owner, tab.keepMounted, tab.title()]).toEqual(["flow.task", "flow", true, "Flow"]);
    expect(r.refusals()).toEqual([]);
  });

  it("nothing where nobody defines the point, and nothing in the diagnostics", () => {
    const fake = fakeHost();
    const r = new Registry();
    r.activate("flow", () => register(r, fake.host));
    r.settle();
    expect(r.list(PROJECT_ITEMS)).toEqual([]);
    expect(r.at("sidebar.tab").some((x) => x.kind === TASK_TAB_KIND)).toBe(true);
    expect(r.refusals()).toEqual([]);
  });
});

describe("what waits on a person, and the overview's block", () => {
  it("a run waiting on a confirm, a hold or an external task is an item; one waiting on its own work, a dry run or a running one is not", () => {
    const runs = [
      summary({ id: "r-confirm", title: "Ship it", phase: { state: "waiting", reason: "confirm", stations: ["approve"] } }),
      summary({ id: "r-held", phase: { state: "waiting", reason: "held", stations: ["a", "b"] }, project_root: "/work/beta" }),
      summary({ id: "r-external", phase: { state: "waiting", reason: "external" } }),
      summary({ id: "r-work", phase: { state: "waiting", reason: "work" } }),
      summary({ id: "r-running", phase: { state: "running" } }),
      summary({ id: "dry-r-1", dry: true, phase: { state: "waiting", reason: "confirm" } }),
    ];
    expect(waitingItems(runs).map((i) => [i.id, i.kind, i.label, i.title, i.detail, i.project_root])).toEqual([
      ["r-confirm", "confirm", "To confirm", "Ship it", "example-review · approve", ROOT],
      ["r-held", "held", "Held", "repo", "example-review · a, b", "/work/beta"],
      ["r-external", "external", "External task", "repo", "example-review", ROOT],
    ]);
  });

  it("counts a project's tasks running, waiting and over", () => {
    const runs = [
      summary({ id: "a", phase: { state: "running" } }),
      summary({ id: "b", phase: { state: "waiting", reason: "confirm" } }),
      summary({ id: "c", status: "finished", phase: { state: "finished" } }),
      summary({ id: "d", status: "cancelled", phase: { state: "cancelled" } }),
      summary({ id: "e", phase: { state: "running" }, project_root: "/work/beta" }),
    ];
    expect(taskCounts(runs, ROOT)).toEqual({ running: 1, waiting: 1, finished: 2 });
  });

  it("registers its source and its block where the Harnix app defines them; the source opens the task", async () => {
    const fake = fakeHost({ "flow.get": view(), "flow.session": { instance: "main", session_id: "general" } });
    const r = new Registry();
    r.activate("system", () => r.definePoint("system.console.tab", { kind: "register" }));
    r.activate("harnix-app", () => {
      r.definePoint(PROJECT_ITEMS, { kind: "register" });
      r.definePoint(PROJECT_PANELS, { kind: "render" });
      r.definePoint(INBOX, { kind: "register" });
      r.register("space", { id: "projects", icon: "folder", title: () => "Projects" }, "harnix-app");
    });
    r.activate("flow", () => register(r, fake.host));
    r.settle();
    const source = r.list<{ id: string; items(): { id: string }[]; subscribe(fn: () => void): () => void; open(item: unknown): Promise<void> }>(INBOX)[0]!;
    expect([source.id, source.owner]).toEqual(["flow.waiting", "flow"]);
    source.subscribe(() => undefined);
    fake.push("flow.runs", {}, { runs: [summary({ phase: { state: "waiting", reason: "confirm" } })] });
    const items = source.items();
    expect(items.map((i) => i.id)).toEqual(["example-review-1"]);
    await source.open(items[0]);
    expect(fake.opened.at(-1)).toEqual({ instance: "main", sessionId: "main-1" });
    expect(fake.tabs.at(-1)).toEqual([TASK_TAB_KIND, { params: { run: "example-review-1" } }]);
    const panel = r.list<{ id: string; order: number; title(): string }>(PROJECT_PANELS)[0]!;
    expect([panel.id, panel.order, panel.title()]).toEqual(["flow.tasks", 30, "Tasks"]);
    expect(r.refusals()).toEqual([]);
  });

  it("the block: the counts, the latest tasks each opening, and 新任务", async () => {
    const fake = fakeHost({ "flow.defs": DEFS });
    const tasks = new ProjectTasks(fake.host, "flow");
    const opened: string[] = [];
    const created = vi.fn();
    const page = render(
      <HostProvider host={fake.host}>
        <TasksPanel tasks={tasks} module="flow" root={ROOT} open={(run) => void opened.push(run.id)} create={created} />
      </HostProvider>,
    );
    expect(page.queryAllByTestId("overview-task")).toEqual([]);
    act(() => fake.push("flow.runs", {}, { runs: [summary({ phase: { state: "waiting", reason: "confirm" } }), summary({ id: "other", project_root: "/work/beta" })] }));
    await settle();
    expect(page.getAllByTestId("overview-task").map((el) => el.dataset.run)).toEqual(["example-review-1"]);
    expect(page.getByTestId("overview-tasks-counts").dataset.waiting).toBe("1");
    fireEvent.click(page.getByTestId("overview-task"));
    expect(opened).toEqual(["example-review-1"]);
    fireEvent.click(page.getByTestId("overview-task-new"));
    expect(created).toHaveBeenCalled();
  });
});

describe("opening a task", () => {
  it("opens the main session where it is shown (show_in_space), in the space in front, and opens the tab; the flow space is left alone", async () => {
    const fake = fakeHost({ "flow.get": view({ show_in_space: "sessions" }) });
    const r = new Registry();
    r.register("space", { id: "projects", icon: "folder", title: () => "Projects" }, "harnix-app");
    const flows = registerSystemFlows(r, { host: fake.host });
    await openTask(fake.host, r, flows, "example-review-1", "example-review");
    expect(fake.host.spaces.switch).not.toHaveBeenCalled();
    expect(fake.opened.at(-1)).toEqual({ instance: "main", sessionId: "main-1" });
    expect(flows.state.get()).toMatchObject({ flow: null, run: null });
    expect(flows.state.from()).toBeNull();
    expect(fake.tabs).toEqual([[TASK_TAB_KIND, { params: { run: "example-review-1" } }]]);
  });

  it("enters the flow space on the run from the projects space when the session is shown nowhere else, opens its main session, closes the panel beside and opens the tab", async () => {
    const fake = fakeHost({ "flow.get": view(), "flow.session": { instance: "main", session_id: "general" } });
    const r = new Registry();
    r.register("space", { id: "projects", icon: "folder", title: () => "Projects" }, "harnix-app");
    const flows = registerSystemFlows(r, { host: fake.host });
    await openTask(fake.host, r, flows, "example-review-1", "example-review");
    expect(fake.host.spaces.switch).toHaveBeenCalledWith("flow");
    expect(fake.opened.at(-1)).toEqual({ instance: "main", sessionId: "main-1" });
    expect(flows.state.get()).toMatchObject({ flow: "example-review", run: "example-review-1" });
    expect(flows.state.detail()).toBeNull();
    expect(flows.state.from()).toEqual({ space: "projects", title: "Projects" });
    expect(fake.tabs).toEqual([[TASK_TAB_KIND, { params: { run: "example-review-1" } }]]);
    // The way back goes to the projects space, once.
    await flows.state.leave();
    expect(fake.host.spaces.switch).toHaveBeenLastCalledWith("projects");
    expect(flows.state.from()).toBeNull();
  });

  it("the row's menu enters the flow space for any task, and the dialog starts a task shown in the space the projects space names", async () => {
    const fake = fakeHost({ "flow.get": view({ show_in_space: "sessions" }), "flow.defs": DEFS, "flow.def": { def: DEF, yaml: "", digest: "a" }, "flow.start": { run: "example-review-2" } });
    const r = new Registry();
    r.activate("harnix-app", () => {
      r.definePoint(PROJECT_ITEMS, { kind: "register" });
      r.register("space", { id: "projects", icon: "folder", title: () => "Projects" }, "harnix-app");
    });
    r.activate("flow", () => register(r, fake.host));
    r.settle();
    const kind = r.list<{ menu(row: unknown, context: unknown): { id: string; run(): Promise<void> }[]; create: { run(context: unknown): Promise<void> } }>(PROJECT_ITEMS)[0]!;
    const [enter] = kind.menu({ id: "example-review-1", title: "t", data: summary({}) }, { root: ROOT, space: "sessions" });
    expect(enter!.id).toBe("flow.task.enter");
    await enter!.run();
    expect(fake.host.spaces.switch).toHaveBeenCalledWith("flow");
    expect(fake.tabs.at(-1)).toEqual([TASK_TAB_KIND, { params: { run: "example-review-1" } }]);
    // The same the plain way, as `enterTask`.
    const flows = registerSystemFlows(new Registry(), { host: fake.host });
    await enterTask(fake.host, r, flows, "example-review-1", "example-review");
    expect(flows.state.get()).toMatchObject({ run: "example-review-1" });

    void kind.create.run({ root: ROOT, space: "sessions" });
    await settle();
    const page = render(<HostProvider host={fake.host}>{fake.dialogs[0]!(() => undefined) as never}</HostProvider>);
    await settle();
    fireEvent.change(page.getByTestId("task-subject"), { target: { value: "the repo" } });
    fireEvent.change(page.getByTestId("task-input-target").querySelector("input")!, { target: { value: "src/" } });
    fireEvent.click(page.getByTestId("task-start"));
    await settle();
    expect(fake.calls.find(([m]) => m === "flow.start")?.[1]).toMatchObject({ project_root: ROOT, show_in_space: "sessions" });
  });
});

describe("the 流程 tab", () => {
  it("draws the run the opener named; a run without params is the module's answer for the session", async () => {
    const get = (p: unknown) => ((p as { run: string }).run === "example-review-1" ? view() : view({ id: "other", session_keys: { "@main": "main/main-9" } }));
    const fake = fakeHost({ "flow.get": get, "flow.runOf": (p: unknown) => ({ run: (p as { session_id: string }).session_id === "main-1" ? "example-review-1" : null }), "flow.sessions": { main: { instance: "main", session_id: "main-1" } } });
    const session = { instance: "main", sessionId: "main-1" };
    const page = render(
      <HostProvider host={fake.host}>
        <TabInfoProvider value={tabInfo(session, { run: "example-review-1" })}>
          <TaskTab host={fake.host} module="flow" />
        </TabInfoProvider>
      </HostProvider>,
    );
    await settle();
    expect(page.getByTestId("flow-task-tab").dataset.run).toBe("example-review-1");
    expect(page.getByTestId("flow-run-panel").dataset.station).toBe("approve");
    expect(page.getByTestId("flow-run-phase").dataset.reason).toBe("confirm");
    expect(page.getAllByTestId(/^flow-run-station-/).map((el) => el.dataset.status)).toEqual(["passed", "working", "pending"]);
    expect(page.getByTestId("flow-gate-result").dataset.status).toBe("pass");
    expect(page.getByTestId("flow-output-dir").dataset.path).toBe("flows/example-review-2026030201");
    expect(page.getByTestId("flow-station-panel").dataset.station).toBe("approve");
    expect(page.getByTestId("flow-confirm")).toBeDefined();
    expect(fake.calls.some(([m]) => m === "flow.runOf")).toBe(false);
    page.unmount();

    const found = render(
      <HostProvider host={fake.host}>
        <TabInfoProvider value={tabInfo(session)}>
          <TaskTab host={fake.host} module="flow" />
        </TabInfoProvider>
      </HostProvider>,
    );
    await settle();
    await settle();
    expect(found.getByTestId("flow-task-tab").dataset.run).toBe("example-review-1");
    expect(fake.calls.find(([m]) => m === "flow.runOf")?.[1]).toEqual({ instance: "main", session_id: "main-1" });
    expect(fake.calls.some(([m]) => m === "flow.list")).toBe(false);
  });

  it("says so for a session that is no task's", async () => {
    const fake = fakeHost({ "flow.runOf": { run: null } });
    const page = render(
      <HostProvider host={fake.host}>
        <TabInfoProvider value={tabInfo({ instance: "main", sessionId: "s-plain" })}>
          <TaskTab host={fake.host} module="flow" />
        </TabInfoProvider>
      </HostProvider>,
    );
    await settle();
    await settle();
    expect(page.getByTestId("flow-task-tab").dataset.state).toBe("none");
  });
});

describe("the new-task dialog", () => {
  it("draws a form from the inputs schema: string, number, boolean, enum, strings, json", () => {
    const fields = fieldsOf({
      type: "object",
      properties: {
        name: { type: "string" },
        depth: { type: "integer", default: 2 },
        ratio: { type: "number" },
        strict: { type: "boolean", default: true },
        mode: { type: "string", enum: ["fast", "slow"], default: "fast" },
        paths: { type: "array", items: { type: "string" }, default: ["a", "b"] },
        extra: { type: "object", default: { k: 1 } },
      },
      required: ["name", "ratio"],
    });
    expect(fields.map((f) => [f.key, f.kind, f.required])).toEqual([
      ["name", "string", true],
      ["depth", "integer", false],
      ["ratio", "number", true],
      ["strict", "boolean", false],
      ["mode", "enum", false],
      ["paths", "strings", false],
      ["extra", "json", false],
    ]);
    expect(fields.map(initialText)).toEqual(["", "2", "", "true", "fast", "a\nb", '{\n  "k": 1\n}']);
    const { inputs, errors } = collectInputs(fields, { name: "x", depth: "3", ratio: "0.5", strict: "false", mode: "slow", paths: "a\n\nb ", extra: '{"k":2}' });
    expect(inputs).toEqual({ name: "x", depth: 3, ratio: 0.5, strict: false, mode: "slow", paths: ["a", "b"], extra: { k: 2 } });
    expect(errors).toEqual({});
    const bad = collectInputs(fields, { name: "", depth: "1.5", ratio: "x", extra: "{" });
    expect(Object.keys(bad.errors).sort()).toEqual(["depth", "extra", "name", "ratio"]);
    // An optional field left blank is not sent.
    expect(collectInputs(fields, { name: "x", ratio: "1" }).inputs).toEqual({ name: "x", ratio: 1 });
    expect(fieldsOf(null)).toEqual([]);
  });

  it("foresees the default output directory, the day's next number, and reads a flow's guard without assuming it", () => {
    const now = Date.parse("2026-03-02T12:00:00");
    const runs = [summary({ created_at: Date.parse("2026-03-02T09:00:00") }), summary({ created_at: Date.parse("2026-03-01T09:00:00") }), summary({ flow: "other", created_at: now })];
    expect(defaultOutputDir("example-review", runs, ROOT, now)).toBe("flows/example-review-2026030202");
    expect(defaultOutputDir("other", [], `${ROOT}/`, now)).toBe("flows/other-2026030201");
    expect(defOptions({ def: { ...DEF, guard: "strict", output_dir: "out/${flow}" } as FlowDef, yaml: "", digest: "a" })).toEqual({ guard: "strict", outputDir: "out/${flow}", workspace: null });
    expect(defOptions(undefined)).toEqual({ guard: null, outputDir: null, workspace: null });
  });

  it("picks a valid flow, needs a subject, warns of a task running, says the project is changed, and starts the task", async () => {
    const fake = fakeHost({ "flow.defs": DEFS, "flow.def": { def: DEF, yaml: "", digest: "a" }, "flow.start": { run: "example-review-2" } });
    const started: [string, string][] = [];
    const running = summary({ title: "nightly review" });
    await newTaskDialog(fake.host, "flow", { root: ROOT, space: "sessions", runs: () => [running] }, (run, flow) => void started.push([run, flow]));
    const close = vi.fn();
    const page = render(<HostProvider host={fake.host}>{fake.dialogs[0]!(close) as never}</HostProvider>);
    await settle();
    expect(page.getByTestId("task-flow-example-review").getAttribute("aria-pressed")).toBe("true");
    expect((page.getByTestId("task-flow-broken") as HTMLButtonElement).disabled).toBe(true);
    expect(page.getByTestId("task-flow-broken").dataset.origin).toBe("user");
    expect(page.getByTestId("task-input-depth").dataset.kind).toBe("integer");
    expect((page.getByTestId("task-input-depth").querySelector("input") as HTMLInputElement).value).toBe("2");
    expect(page.getByTestId("task-running-warning").textContent).toContain("nightly review");
    expect(page.getByTestId("task-modifies")).toBeDefined();
    expect((page.getByTestId("task-worktree") as HTMLInputElement).disabled).toBe(true);
    expect((page.getByTestId("task-output-dir") as HTMLInputElement).placeholder).toMatch(/^flows\/example-review-\d{10}$/);
    const start = page.getByTestId("task-start") as HTMLButtonElement;
    expect(start.disabled).toBe(true);
    fireEvent.change(page.getByTestId("task-subject"), { target: { value: "the repo" } });
    // The required input is still blank: not yet.
    expect(start.disabled).toBe(true);
    fireEvent.change(page.getByTestId("task-input-target").querySelector("input")!, { target: { value: "src/" } });
    fireEvent.change(page.getByTestId("task-title"), { target: { value: "Review src" } });
    expect(start.disabled).toBe(false);
    fireEvent.click(start);
    await settle();
    expect(fake.calls.find(([m]) => m === "flow.start")?.[1]).toEqual({ flow: "example-review", subject: "the repo", project_root: ROOT, inputs: { depth: 2, target: "src/" }, title: "Review src", show_in_space: "sessions" });
    expect(close).toHaveBeenCalledWith(true);
    expect(started).toEqual([["example-review-2", "example-review"]]);
    // The flows a task of this project may run, the project's own among them.
    expect(fake.calls.find(([m]) => m === "flow.defs")?.[1]).toEqual({ project: ROOT });
  });

  it("offers the project's own flows with their origin, and an untrusted project's as waiting on trust", async () => {
    const defs = [
      ...DEFS,
      { name: "house", title: "House review", digest: "h", origin: "project", valid: true, inputs: { type: "object" } },
      { name: "pending", title: "", digest: "", origin: "project", valid: false, reason: "untrusted", inputs: { type: "object" } },
    ];
    const fake = fakeHost({ "flow.defs": defs, "flow.def": { def: DEF, yaml: "", digest: "a" } });
    await newTaskDialog(fake.host, "flow", { root: ROOT, runs: () => [] }, () => undefined);
    const page = render(<HostProvider host={fake.host}>{fake.dialogs[0]!(vi.fn()) as never}</HostProvider>);
    await settle();
    expect(page.getByTestId("task-flow-house").dataset.origin).toBe("project");
    expect(page.getByTestId("task-flow-house").textContent).toContain("this project's");
    const pending = page.getByTestId("task-flow-pending") as HTMLButtonElement;
    expect(pending.disabled).toBe(true);
    expect(pending.dataset.reason).toBe("untrusted");
    expect(pending.textContent).toContain("trust the project first");
  });
});

describe("a task in a worktree", () => {
  it("sends a workspace only when the toggle was touched in a git repository; says why not otherwise", () => {
    expect(workspaceParam(true, null)).toBeUndefined();
    expect(workspaceParam(true, true)).toBe("worktree");
    expect(workspaceParam(true, false)).toBe("project");
    expect(workspaceParam(false, true)).toBeUndefined();
    expect(noWorktreeText({ git: true, branch: "main" })).toBeNull();
    expect(noWorktreeText(undefined)).toBeNull();
    expect(noWorktreeText({ git: false, reason: "not_a_repository" })).toBe("Needs a git repository: this project is not one.");
    expect(noWorktreeText({ git: false, reason: "odd", detail: "git said so" })).toBe("git said so");
  });

  const dialog = async (answers: Record<string, unknown>) => {
    const fake = fakeHost({ "flow.defs": DEFS, "flow.def": { def: DEF, yaml: "", digest: "a" }, "flow.start": { run: "example-review-2" }, ...answers });
    await newTaskDialog(fake.host, "flow", { root: ROOT, runs: () => [] }, () => undefined);
    const page = render(<HostProvider host={fake.host}>{fake.dialogs[0]!(vi.fn()) as never}</HostProvider>);
    await settle();
    fireEvent.change(page.getByTestId("task-subject"), { target: { value: "the repo" } });
    fireEvent.change(page.getByTestId("task-input-target").querySelector("input")!, { target: { value: "src/" } });
    return { fake, page, toggle: page.getByTestId("task-worktree") as HTMLInputElement };
  };
  const sent = (fake: ReturnType<typeof fakeHost>) => fake.calls.find(([m]) => m === "flow.start")?.[1] as Record<string, unknown>;

  it("the dialog's toggle: on in a git repository, asked of the module, and sent when turned on", async () => {
    const { fake, page, toggle } = await dialog({ "flow.workspace_info": { git: true, branch: "main" } });
    expect(fake.calls.find(([m]) => m === "flow.workspace_info")?.[1]).toEqual({ project_root: ROOT });
    expect(toggle.disabled).toBe(false);
    expect(toggle.checked).toBe(false);
    expect(page.queryByTestId("task-worktree-reason")).toBeNull();
    fireEvent.click(toggle);
    expect(page.getByTestId("task-modifies").dataset.worktree).toBe("true");
    fireEvent.click(page.getByTestId("task-start"));
    await settle();
    expect(sent(fake).workspace).toBe("worktree");
  });

  it("the dialog's toggle: a flow that asks for a worktree starts it on, and turned off sends project", async () => {
    const def = { def: { ...DEF, workspace: "worktree" }, yaml: "", digest: "a" };
    const { fake, page, toggle } = await dialog({ "flow.workspace_info": { git: true, branch: "main" }, "flow.def": def });
    expect(toggle.checked).toBe(true);
    fireEvent.click(toggle);
    expect(toggle.checked).toBe(false);
    fireEvent.click(page.getByTestId("task-start"));
    await settle();
    expect(sent(fake).workspace).toBe("project");
  });

  it("the dialog's toggle: off with the reason in a project that is no git repository", async () => {
    const def = { def: { ...DEF, workspace: "worktree" }, yaml: "", digest: "a" };
    const { fake, page, toggle } = await dialog({ "flow.workspace_info": { git: false, reason: "not_a_repository", detail: "…" }, "flow.def": def });
    expect(toggle.disabled).toBe(true);
    expect(toggle.checked).toBe(false);
    expect(page.getByTestId("task-worktree-reason").textContent).toBe("Needs a git repository: this project is not one.");
    expect(page.getByTestId("task-worktree-needed")).toBeDefined();
    fireEvent.click(page.getByTestId("task-start"));
    await settle();
    expect(sent(fake).workspace).toBeUndefined();
  });

  const over = (worktree: WorktreeRec, status: RunView["record"]["status"] = "finished"): RunView =>
    view({ status, workspace: "worktree", worktree }, { state: status === "active" ? "running" : (status as "finished") });

  it("the 流程 tab: the branch while it runs, nothing to decide yet", async () => {
    const fake = fakeHost({ "flow.workspace_info": { git: true, branch: "main" } });
    const page = render(
      <HostProvider host={fake.host}>
        <WorktreeLine view={over(WORKTREE, "active")} module="flow" />
      </HostProvider>,
    );
    await settle();
    expect(page.getByTestId("flow-worktree-branch").textContent).toBe("harnix/example-review-1");
    expect(page.queryByTestId("flow-worktree-actions")).toBeNull();
    expect(worktreeOpen(over(WORKTREE, "active"))).toBe(false);
    expect(worktreeOpen(over(WORKTREE))).toBe(true);
    expect(worktreeOpen(over({ ...WORKTREE, outcome: { kind: "kept", at: 1, by: { kind: "device", id: "d" } } }))).toBe(true);
    expect(worktreeOpen(over({ ...WORKTREE, outcome: { kind: "discarded", at: 1, by: { kind: "device", id: "d" } } }))).toBe(false);
  });

  it("the 流程 tab, the run over: merge into the current branch and discard after asking, keep at once", async () => {
    const fake = fakeHost({ "flow.workspace_info": { git: true, branch: "main" } });
    const page = render(
      <HostProvider host={fake.host}>
        <WorktreeLine view={over(WORKTREE)} module="flow" />
      </HostProvider>,
    );
    await settle();
    expect(page.getByTestId("flow-worktree-merge").textContent).toBe("Merge into main");
    fireEvent.click(page.getByTestId("flow-worktree-merge"));
    await settle();
    expect(fake.confirms[0]![0]).toContain("harnix/example-review-1 into main");
    expect(fake.calls.filter(([m]) => m === "flow.worktree.merge")).toEqual([["flow.worktree.merge", { run: "example-review-1" }]]);
    fireEvent.click(page.getByTestId("flow-worktree-keep"));
    await settle();
    expect(fake.confirms.length).toBe(1);
    expect(fake.calls.some(([m]) => m === "flow.worktree.keep")).toBe(true);
    fireEvent.click(page.getByTestId("flow-worktree-discard"));
    await settle();
    expect(fake.confirms[1]![1]).toMatchObject({ danger: true });
    expect(fake.calls.some(([m]) => m === "flow.worktree.discard")).toBe(true);
  });

  it("the 流程 tab: nothing is done when the question is declined; a conflict is said; what was decided is shown", async () => {
    const conflict = () => Promise.reject(Object.assign(new Error("merging harnix/example-review-1 into the project conflicts in a.txt"), { code: -31007 }));
    const fake = fakeHost({ "flow.workspace_info": { git: true, branch: "main" }, "flow.worktree.merge": conflict }, false);
    const page = render(
      <HostProvider host={fake.host}>
        <WorktreeLine view={over(WORKTREE)} module="flow" />
      </HostProvider>,
    );
    await settle();
    fireEvent.click(page.getByTestId("flow-worktree-discard"));
    await settle();
    expect(fake.calls.some(([m]) => m === "flow.worktree.discard")).toBe(false);
    page.unmount();

    const yes = fakeHost({ "flow.workspace_info": { git: true, branch: "main" }, "flow.worktree.merge": conflict });
    const again = render(
      <HostProvider host={yes.host}>
        <WorktreeLine view={over(WORKTREE)} module="flow" />
      </HostProvider>,
    );
    await settle();
    fireEvent.click(again.getByTestId("flow-worktree-merge"));
    await settle();
    expect(again.getByTestId("flow-worktree-error").textContent).toContain("conflicts in a.txt");
    again.unmount();

    const decided = render(
      <HostProvider host={yes.host}>
        <WorktreeLine view={over({ ...WORKTREE, outcome: { kind: "merged", at: 1, by: { kind: "device", id: "d" }, into: "main", how: "fast_forward" } })} module="flow" />
      </HostProvider>,
    );
    await settle();
    expect(decided.getByTestId("flow-worktree").dataset.outcome).toBe("merged");
    expect(decided.getByTestId("flow-worktree-outcome").textContent).toBe("merged into main");
    expect(decided.queryByTestId("flow-worktree-actions")).toBeNull();
  });
});

describe("the points it defines: a task row's badges, a flow's other ways to start", () => {
  const withPoints = (fake: ReturnType<typeof fakeHost>) => {
    const r = new Registry();
    r.activate("flow", () => register(r, fake.host));
    r.settle();
    return r;
  };

  it("defines both; a badge is drawn in the row's badges after the clock, one that throws loses only itself", async () => {
    const fake = fakeHost({ "flow.defs": DEFS });
    const r = withPoints(fake);
    expect(r.definitions().filter((d) => d.owner === "flow").map((d) => [d.name, d.kind])).toEqual([
      [TASK_BADGE, "render"],
      [START_ACTION, "register"],
    ]);
    const asked: string[] = [];
    const badge: TaskBadge = { id: "test.next", render: (run) => (asked.push(run.id), <span data-testid="test-next">next</span>) };
    const broken: TaskBadge = { id: "test.broken", render: () => { throw new Error("no"); } };
    r.activate("test", () => {
      r.register(TASK_BADGE, badge, "test");
      r.register(TASK_BADGE, broken, "test");
    });
    const fail = vi.spyOn(r, "fail");
    const tasks = new ProjectTasks(fake.host, "flow", r);
    const run = summary({ id: "example-review-9", trigger: { kind: "schedule", schedule: "s1", fire_at: 1 } });
    const page = render(<HostProvider host={fake.host}><TaskRow run={run} module="flow" tasks={tasks} /></HostProvider>);
    await settle();
    const badges = page.getByTestId("task-badges");
    expect(badges.querySelector("svg.hx-flow-task-clock")).not.toBeNull();
    expect(badges.lastElementChild?.getAttribute("data-testid")).toBe("test-next");
    expect(asked).toContain("example-review-9");
    expect(fail).toHaveBeenCalledWith(TASK_BADGE, expect.objectContaining({ id: "test.broken" }), expect.any(Error));
  });

  it("the new-task dialog offers another way to start with what was filled in, the dialog closed first", async () => {
    const fake = fakeHost({ "flow.defs": DEFS, "flow.def": { def: DEF, yaml: "", digest: "a" } });
    const r = withPoints(fake);
    const drafts: StartDraft[] = [];
    const action: StartAction = { id: "test.later", label: () => "Later…", run: (draft) => void drafts.push(draft) };
    r.activate("test", () => r.register(START_ACTION, action, "test"));
    await newTaskDialog(fake.host, "flow", { root: ROOT, space: "sessions", runs: () => [], registry: r }, () => undefined);
    const close = vi.fn();
    const page = render(<HostProvider host={fake.host}>{fake.dialogs[0]!(close) as never}</HostProvider>);
    await settle();
    const later = page.getByTestId("task-action-test.later") as HTMLButtonElement;
    expect(later.textContent).toBe("Later…");
    // A required input still blank: nothing to hand over yet.
    expect(later.disabled).toBe(true);
    fireEvent.change(page.getByTestId("task-input-target").querySelector("input")!, { target: { value: "src/" } });
    fireEvent.change(page.getByTestId("task-title"), { target: { value: "Nightly" } });
    expect(later.disabled).toBe(false);
    fireEvent.click(later);
    await settle();
    expect(close).toHaveBeenCalledWith(null);
    expect(drafts).toEqual([{ flow: "example-review", inputs: { depth: 2, target: "src/" }, title: "Nightly", project_root: ROOT, show_in_space: "sessions" }]);
    expect(fake.calls.some(([m]) => m === "flow.start")).toBe(false);
  });

  it("without a registry, or with nothing registered, the dialog offers nothing more", async () => {
    const fake = fakeHost({ "flow.defs": DEFS, "flow.def": { def: DEF, yaml: "", digest: "a" } });
    await newTaskDialog(fake.host, "flow", { root: ROOT, runs: () => [], registry: withPoints(fake) }, () => undefined);
    const page = render(<HostProvider host={fake.host}>{fake.dialogs[0]!(vi.fn()) as never}</HostProvider>);
    await settle();
    expect(page.container.querySelector('[data-testid^="task-action-"]')).toBeNull();
  });
});
