/**
 * A task in a project: the rows the projects tree gets (the project's runs, dry runs left out,
 * by activity), the kind registered where the projects space defines its point and the 流程 tab
 * type, the new-task dialog's form from an `inputs` schema and what it sends to `start`, a task
 * opened (the flow space entered on it, the main session, the tab), and the tab drawing the run.
 */

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { HostProvider, Registry, TabInfoProvider, setLocale, type HostApi, type HostSessionRef, type TabInfo } from "@harnix/react";

import { collectInputs, defOptions, defaultOutputDir, fieldsOf, initialText, newTaskDialog } from "../src/NewTask";
import { register } from "../src/register";
import { currentStation, taskStep } from "../src/RunPanel";
import { registerSystemFlows } from "../src/space";
import { registerFlowStrings } from "../src/strings";
import { INBOX, PROJECT_ITEMS, PROJECT_PANELS, ProjectTasks, TASK_KIND, TASK_TAB_KIND, TaskRow, TasksPanel, openTask, taskCounts, taskRows, waitingItems } from "../src/task";
import { TaskTab } from "../src/TaskTab";
import type { DefSummary, FlowDef, RunSummary, RunView } from "../src/types";

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
function fakeHost(answers: Record<string, unknown> = {}) {
  const calls: [string, unknown][] = [];
  const topics = new Map<string, Set<(v: unknown) => void>>();
  const values = new Map<string, unknown>();
  const changes = new Set<(s: HostSessionRef | null) => void>();
  const opened: HostSessionRef[] = [];
  const tabs: [string, unknown][] = [];
  const dialogs: ((close: (r: unknown) => void) => unknown)[] = [];
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
    dialogs: { open: vi.fn(async (render: (close: (r: unknown) => void) => unknown) => void dialogs.push(render)) },
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
  it("enters the flow space on the run from the projects space, opens its main session, closes the panel beside and opens the tab", async () => {
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
});

describe("the 流程 tab", () => {
  it("draws the run the opener named, remembered for the session; a run without params is found among the runs", async () => {
    const get = (p: unknown) => ((p as { run: string }).run === "example-review-1" ? view() : view({ id: "other", session_keys: { "@main": "main/main-9" } }));
    const fake = fakeHost({ "flow.get": get, "flow.list": { runs: [summary({ id: "other" }), summary({})] }, "flow.sessions": { main: { instance: "main", session_id: "main-1" } } });
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
    expect(localStorage.getItem("harnix.flow.task.main/main-1")).toBe("example-review-1");
    page.unmount();

    localStorage.clear();
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
    expect(fake.calls.some(([m, p]) => m === "flow.list" && (p as { limit: number }).limit === 20)).toBe(true);
  });

  it("says so for a session that is no task's", async () => {
    const fake = fakeHost({ "flow.list": { runs: [] } });
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
    await newTaskDialog(fake.host, "flow", { root: ROOT, runs: () => [running] }, (run, flow) => void started.push([run, flow]));
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
    expect(fake.calls.find(([m]) => m === "flow.start")?.[1]).toEqual({ flow: "example-review", subject: "the repo", project_root: ROOT, inputs: { depth: 2, target: "src/" }, title: "Review src" });
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
