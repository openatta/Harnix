/**
 * The flow space of xops-03 §6 against a fake host: what is selected on the right is the
 * conversation in the middle (a flow's own, a run's main session, the general one), the line over
 * the composer, the right column's tabs with their first rows, a run's state, time, tokens and
 * sub-agents, the guide, a dry run's controls, and the message a dry run's session task would have
 * been sent.
 */

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { NestProvider, Registry, setLocale, type HostSessionRef, type NestHost } from "@harnix/react";

import { toolTarget } from "../src/model";
import { registerFlowSpace } from "../src/space";
import { ToolTargetLine } from "../src/ToolTarget";
import { FlowSpaceState } from "../src/state";
import { StationPanel } from "../src/StationPanel";
import { registerFlowStrings } from "../src/strings";
import { DryControls, FlowsTab, FocusNotice, ReferenceTab, RunsTab, type FlowTabs } from "../src/tabs";
import type { DefInfo, DefSummary, FlowDef, RunView } from "../src/types";

afterEach(cleanup);
setLocale("en");
registerFlowStrings(new Registry());

const DEF: FlowDef = {
  name: "example-review",
  title: "Example: review and approve",
  stations: [
    { id: "review", executor: { kind: "session", scene: "flow-chat" }, skill: "review", output: { type: "object" }, exit: [{ kind: "output" }] },
    { id: "approve", exit: [{ kind: "confirm" }], rework_to: ["review"] },
    { id: "sign", needs: ["approve"], executor: { kind: "external", who: "clerk" }, output: { type: "object" }, exit: [{ kind: "output" }] },
  ],
};

const DEFS: DefSummary[] = [
  { name: "example-review", title: "Example: review and approve", digest: "aaaa1111", origin: "builtin", valid: true },
  { name: "broken", title: "Broken", digest: "bbbb", origin: "user", valid: false },
];

const INFO: DefInfo = { def: DEF, yaml: "name: example-review\n", digest: "aaaa1111", skills: { review: "Look at the subject." }, origin: "builtin", valid: true, problems: [] };

const DRY: RunView = {
  record: {
    id: "dry-example-review-1",
    flow: "example-review",
    def_digest: "aaaa1111",
    subject: "dry-run",
    status: "active",
    rev: 2,
    created_at: 2,
    created_by: { kind: "device", id: "d" },
    def: DEF,
    stations: {
      review: {
        status: "passed",
        round: 1,
        tasks: [
          {
            id: "review",
            status: "submitted",
            attempt: 1,
            output: { summary: "" },
            preview: { round: 1, attempt: 1, scene: "flow-chat", delivery: "flow_submit", message: "# Flow protocol\n\n## Skill `review`\nLook at the subject." },
          },
        ],
      },
      approve: { status: "working", round: 1, tasks: [] },
      sign: { status: "pending", round: 0 },
    },
  },
};

/** A host whose topics can be pushed to, with an open session, a front space and a right column. */
function fakeHost(answers: Record<string, unknown> = {}) {
  const calls: [string, unknown][] = [];
  const topics = new Map<string, Set<(v: unknown) => void>>();
  const values = new Map<string, unknown>();
  const changes = new Set<(s: HostSessionRef | null) => void>();
  const tabFns = new Set<(tab: string) => void>();
  let current: HostSessionRef | null = { instance: "system", sessionId: "s1" };
  let front = "flow";
  let tab = "flow.flows";
  const keyOf = (topic: string, params?: Record<string, unknown>) => `${topic} ${JSON.stringify(params ?? {})}`;
  const opened: HostSessionRef[] = [];
  const inserted: [string, unknown][] = [];
  const setCurrent = (session: HostSessionRef | null) => {
    current = session;
    for (const fn of [...changes]) fn(session);
  };
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
        setCurrent(ref);
      }),
    },
    spaces: { current: () => front },
    details: {
      open: vi.fn(),
      tab: () => tab,
      onTab: (fn: (tab: string) => void) => {
        tabFns.add(fn);
        return () => tabFns.delete(fn);
      },
    },
    composer: { insert: vi.fn((text: string, options?: unknown) => inserted.push([text, options])) },
    redraw: vi.fn(),
    t: (k: string) => k,
  } as unknown as NestHost;
  return {
    host,
    calls,
    opened,
    inserted,
    push(topic: string, params: Record<string, unknown> | undefined, value: unknown) {
      const key = keyOf(topic, params);
      values.set(key, value);
      for (const fn of [...(topics.get(key) ?? [])]) fn(value);
    },
    open(session: HostSessionRef | null, space = "flow") {
      front = space;
      setCurrent(session);
    },
    tab(next: string) {
      tab = next;
      for (const fn of [...tabFns]) fn(next);
    },
  };
}

/** `session {flow?}`: `f-<flow>` for a flow's conversation, `general` for the general one. */
const conversations = (p: { flow?: string }) => ({ instance: "system", session_id: p?.flow ? `f-${p.flow}` : "general", flow: p?.flow ?? null });
const ids = (refs: HostSessionRef[]) => refs.map((r) => r.sessionId);
const settle = () => act(async () => {});

describe("what is selected on the right is what the middle talks to", () => {
  it("a flow: its own conversation; nothing: the general one; no focus is sent", async () => {
    const fake = fakeHost({ "app.flow.session": conversations });
    const state = new FlowSpaceState();
    state.attach(fake.host);
    await settle();
    state.selectFlow("example-review");
    await settle();
    expect(ids(fake.opened)).toEqual(["f-example-review"]);
    expect(fake.calls).toContainEqual(["app.flow.session", { flow: "example-review" }]);
    state.selectFlow(null);
    await settle();
    expect(ids(fake.opened)).toEqual(["f-example-review", "general"]);
    expect(fake.calls.some(([m]) => m === "app.flow.focus")).toBe(false);
  });

  it("a run: its main session; the tab in front decides; the reference tab is the general conversation", async () => {
    const fake = fakeHost({
      "app.flow.session": conversations,
      "app.flow.get": { record: { id: "example-review-1", flow: "example-review", session_keys: { "@main": "system/main-1" } } },
    });
    const state = new FlowSpaceState();
    state.attach(fake.host);
    state.selectFlow("example-review");
    await settle();
    act(() => fake.tab("flow.runs"));
    await settle();
    // Nothing selected in the runs tab: the general conversation.
    expect(ids(fake.opened).at(-1)).toBe("general");
    state.showRun("example-review-1");
    await settle();
    expect(fake.opened.at(-1)).toEqual({ instance: "system", sessionId: "main-1" });
    act(() => fake.tab("flow.flows"));
    await settle();
    expect(ids(fake.opened).at(-1)).toBe("f-example-review");
    act(() => fake.tab("flow.reference"));
    await settle();
    expect(ids(fake.opened).at(-1)).toBe("general");
  });

  it("puts the middle on the selection when the space comes to the front", async () => {
    const fake = fakeHost({ "app.flow.session": conversations });
    act(() => fake.open({ instance: "system", sessionId: "x" }, "sessions"));
    const state = new FlowSpaceState();
    state.attach(fake.host);
    state.selectFlow("example-review");
    await settle();
    expect(fake.opened).toEqual([]);
    act(() => fake.open({ instance: "system", sessionId: "last" }, "flow"));
    await settle();
    expect(ids(fake.opened)).toEqual(["f-example-review"]);
  });

  it("does nothing while another space is in front, and drops an answer a later switch overtook", async () => {
    let slow: (v: unknown) => void = () => {};
    const fake = fakeHost({
      "app.flow.session": (p: { flow?: string }) => (p.flow === "slow" ? new Promise((r) => (slow = r)) : conversations(p)),
    });
    const state = new FlowSpaceState();
    state.attach(fake.host);
    act(() => fake.open({ instance: "system", sessionId: "x" }, "sessions"));
    state.selectFlow("example-review");
    await settle();
    expect(fake.opened).toEqual([]);
    act(() => fake.open({ instance: "system", sessionId: "s1" }, "flow"));
    state.selectFlow("slow");
    state.selectFlow("broken");
    await settle();
    await act(async () => slow(conversations({ flow: "slow" })));
    expect(ids(fake.opened)).toEqual(["f-broken"]);
  });

  it("follows the topic: a flow the general conversation created is selected, a deleted one clears it", async () => {
    const fake = fakeHost();
    const state = new FlowSpaceState();
    state.attach(fake.host);
    const topic = { instance: "system", session_id: "s1" };
    act(() => fake.push("app.flow.focus", topic, { flow: null }));
    expect(state.get().flow).toBe(null);
    act(() => fake.push("app.flow.focus", topic, { flow: "authored" }));
    expect(state.get()).toMatchObject({ flow: "authored", run: null });
    expect(state.focusOf()).toBe("authored");
    act(() => fake.push("app.flow.focus", topic, { flow: null }));
    expect(state.get().flow).toBe(null);
    // A run selected: the topic does not take the selection over.
    state.showRun("r-1");
    act(() => fake.push("app.flow.focus", topic, { flow: "other" }));
    expect(state.get()).toMatchObject({ flow: null, run: "r-1" });
  });
});

describe("the line over the composer", () => {
  it("names the conversation's flow, or says it is the general one", async () => {
    const fake = fakeHost();
    fake.push("app.flow.defs", {}, DEFS);
    const state = new FlowSpaceState();
    render(
      <NestProvider host={fake.host}>
        <FocusNotice state={state} session={{ instance: "system", sessionId: "s1" }} />
      </NestProvider>,
    );
    const notice = screen.getByTestId("flow-focus-notice");
    expect(notice.dataset.flow).toBe("");
    expect(notice.textContent).toContain("The general conversation");
    act(() => fake.push("app.flow.focus", { instance: "system", session_id: "s1" }, { flow: "example-review" }));
    expect(screen.getByTestId("flow-focus-notice").dataset.flow).toBe("example-review");
    expect(screen.getByTestId("flow-focus-notice").textContent).toContain("This conversation is for Example: review and approve");
  });

  it("is what the space gives the shell", () => {
    const r = new Registry();
    const flow = registerFlowSpace(r);
    const space = r.at("space")[0]!;
    expect(space.notice?.()).toContain("general conversation");
    flow.state.selectFlow("example-review");
    expect(space.notice?.()).toContain("example-review");
    expect(flow.state.tabs).toEqual(flow.tabs);
  });
});

const TABS: FlowTabs = { flows: "flow.flows", runs: "flow.runs", reference: "flow.reference" };

describe("流程", () => {
  it("lists the flows with their origin and validity, filtered and sorted; adds in the general conversation", async () => {
    const fake = fakeHost({ "app.flow.session": conversations });
    fake.push("app.flow.defs", {}, [...DEFS, { name: "alpha", title: "Zed", digest: "z", origin: "override", valid: true }]);
    const state = new FlowSpaceState();
    render(
      <NestProvider host={fake.host}>
        <FlowsTab state={state} tabs={TABS} />
      </NestProvider>,
    );
    const names = () => [...screen.getByTestId("flow-def-list").querySelectorAll("button")].map((b) => b.dataset.testid);
    expect(names()).toEqual(["flow-def-alpha", "flow-def-broken", "flow-def-example-review"]);
    expect(screen.getByTestId("flow-def-example-review").dataset.origin).toBe("builtin");
    expect(screen.getByTestId("flow-def-broken").dataset.valid).toBe("false");
    fireEvent.change(screen.getByTestId("flow-sort-flows"), { target: { value: "title" } });
    expect(names()).toEqual(["flow-def-broken", "flow-def-example-review", "flow-def-alpha"]);
    fireEvent.change(screen.getByTestId("flow-filter-valid"), { target: { value: "valid" } });
    expect(names()).toEqual(["flow-def-example-review", "flow-def-alpha"]);
    fireEvent.change(screen.getByTestId("flow-filter-origin"), { target: { value: "override" } });
    expect(names()).toEqual(["flow-def-alpha"]);
    fireEvent.change(screen.getByTestId("flow-filter-origin"), { target: { value: "" } });
    fireEvent.change(screen.getByTestId("flow-filter-name"), { target: { value: "review" } });
    expect(names()).toEqual(["flow-def-example-review"]);

    state.attach(fake.host);
    await act(async () => fireEvent.click(screen.getByTestId("flow-add")));
    expect(ids(fake.opened).at(-1)).toBe("general");
    expect(fake.inserted).toEqual([["Create a new flow: ", { session: { instance: "system", sessionId: "general" } }]]);
  });

  it("draws the selected one: graph, skills, read-only YAML; a dry run started here is shown in the runs tab", async () => {
    const fake = fakeHost({ "app.flow.def": INFO, "app.flow.session": conversations, "app.flow.dry_run.start": { run: "dry-example-review-2" } });
    fake.push("app.flow.defs", {}, DEFS);
    const state = new FlowSpaceState();
    render(
      <NestProvider host={fake.host}>
        <FlowsTab state={state} tabs={TABS} />
      </NestProvider>,
    );
    expect(screen.getByTestId("flow-pick-hint")).toBeTruthy();
    await act(async () => fireEvent.click(screen.getByTestId("flow-def-example-review")));
    expect(ids(fake.opened)).toEqual(["f-example-review"]);
    expect(screen.getByTestId("flow-detail").dataset.digest).toBe("aaaa1111");
    expect(screen.getByTestId("flow-graph").dataset.mode).toBe("definition");
    expect(screen.getByTestId("flow-skill-body-review").textContent).toContain("Look at the subject.");
    expect(screen.getByTestId("flow-yaml-text").textContent).toBe("name: example-review\n");
    // Nothing here edits a definition.
    expect(document.querySelector("textarea")).toBeNull();
    expect(fake.calls.some(([m]) => /app\.flow\.(save|delete|skill|validate)/.test(m))).toBe(false);
    fireEvent.click(screen.getByTestId("flow-node-review"));
    expect(screen.getByTestId("flow-station-panel").dataset.station).toBe("review");

    await act(async () => fireEvent.click(screen.getByTestId("flow-dry-start")));
    expect(fake.calls).toContainEqual(["app.flow.dry_run.start", { flow: "example-review" }]);
    expect(state.get().run).toBe("dry-example-review-2");
    expect(fake.host.details.open).toHaveBeenCalledWith("flow.runs");
  });

  it("a saved flow is read again: the list's digest changed", async () => {
    const fake = fakeHost({ "app.flow.def": INFO });
    fake.push("app.flow.defs", {}, DEFS);
    const state = new FlowSpaceState();
    state.selectFlow("example-review");
    render(
      <NestProvider host={fake.host}>
        <FlowsTab state={state} tabs={TABS} />
      </NestProvider>,
    );
    await settle();
    const reads = () => fake.calls.filter(([m]) => m === "app.flow.def").length;
    const before = reads();
    act(() => fake.push("app.flow.defs", {}, [{ ...DEFS[0]!, digest: "cccc", title: "Changed" }, DEFS[1]!]));
    await settle();
    expect(reads()).toBe(before + 1);
    expect(screen.getByTestId("flow-def-example-review").textContent).toContain("Changed");
  });
});

const usage = (n: number) => ({ input_tokens: n, output_tokens: 0, turns: 1 });

const LIVE: RunView = {
  record: {
    ...DRY.record,
    id: "example-review-1",
    subject: "repo",
    created_at: 1_000,
    session_keys: { "@main": "system/main-1" },
    stations: {
      review: { status: "passed", round: 1, entered_at: 1_000, left_at: 4_000, tasks: [{ id: "review", status: "submitted", attempt: 1, started_at: 1_000, ended_at: 4_000, usage: usage(1500) }] },
      approve: { status: "working", round: 1, entered_at: 4_000, tasks: [] },
      sign: { status: "pending", round: 0 },
    },
  },
  unmet: { approve: [{ kind: "confirm", status: "wait" }] },
  phase: { state: "waiting", reason: "confirm", stations: ["approve"] },
};

describe("运行", () => {
  const RUNS = [
    { id: "example-review-1", flow: "example-review", subject: "repo", status: "active", created_at: 3, phase: { state: "waiting", reason: "confirm", stations: ["approve"] }, usage: usage(1500) },
    { id: "example-review-2", flow: "example-review", subject: "x", status: "active", created_at: 5, phase: { state: "pausing", stations: ["review"] }, usage: usage(200) },
    { id: "other-1", flow: "other", subject: "x", status: "finished", created_at: 4, finished_at: 9, phase: { state: "finished" } },
  ];

  it("every flow's runs and dry runs, filtered by state, flow and kind, sorted by start or tokens", () => {
    const fake = fakeHost();
    fake.push("app.flow.defs", {}, DEFS);
    fake.push("app.flow.runs", {}, RUNS);
    fake.push("app.flow.dry_runs", {}, [{ id: "dry-example-review-1", flow: "example-review", status: "active", created_at: 2 }]);
    const state = new FlowSpaceState();
    state.selectFlow("example-review");
    render(
      <NestProvider host={fake.host}>
        <RunsTab state={state} tabs={TABS} />
      </NestProvider>,
    );
    const rows = () => [...screen.getByTestId("flow-run-list").querySelectorAll("button")].map((b) => b.dataset.testid!.replace("flow-run-", ""));
    // The selected flow does not filter the runs: every flow's are here.
    expect(rows()).toEqual(["example-review-2", "other-1", "example-review-1", "dry-example-review-1"]);
    expect(screen.getByTestId("flow-run-dry-example-review-1").dataset.dry).toBe("true");
    expect(screen.getByTestId("flow-run-example-review-2").dataset.phase).toBe("pausing");
    expect(screen.getByTestId("flow-run-example-review-2").querySelector(".hx-flow-spin")).toBeTruthy();
    expect(screen.getByTestId("flow-run-example-review-1").textContent).toContain("Waiting · to be confirmed");
    expect(screen.getByTestId("flow-run-example-review-1").textContent).toContain("1.5k tokens");
    // No "start" or "add" here.
    expect(screen.queryByTestId("flow-start-open")).toBeNull();
    expect(screen.queryByTestId("flow-add")).toBeNull();

    fireEvent.change(screen.getByTestId("flow-sort-runs"), { target: { value: "tokens" } });
    expect(rows()[0]).toBe("example-review-1");
    fireEvent.change(screen.getByTestId("flow-filter-kind"), { target: { value: "dry" } });
    expect(rows()).toEqual(["dry-example-review-1"]);
    fireEvent.change(screen.getByTestId("flow-filter-kind"), { target: { value: "" } });
    fireEvent.change(screen.getByTestId("flow-filter-flow"), { target: { value: "other" } });
    expect(rows()).toEqual(["other-1"]);
    fireEvent.change(screen.getByTestId("flow-filter-flow"), { target: { value: "" } });
    fireEvent.change(screen.getByTestId("flow-filter-state"), { target: { value: "waiting" } });
    expect(rows()).toEqual(["example-review-1"]);
  });

  it("a run: its state, time and tokens per station, pause, and its sub-agents and tasks to open", async () => {
    const fake = fakeHost({
      "app.flow.get": LIVE,
      "app.flow.sessions": {
        main: { instance: "system", session_id: "main-1" },
        agents: [{ agent_session_id: "agent-1", instance: "system", agent_type: "flow-agent", description: "flow:example-review-1/review/review", flow: { station: "review", task: "review" }, status: "done" }],
        own: [{ station: "sign", task: "sign", session: { instance: "box", session_id: "own-1" } }],
        tasks: [{ task_id: "t1", kind: "agent", status: "done", started_ms: 1, ended_ms: 2001, detail: { agent_type: "reviewer", description: "flow:example-review-1/review/review" }, flow: { station: "review", task: "review" } }],
        sidechains: [{ session_id: "side-1", instance: "system", name: "a side chain" }],
      },
    });
    fake.push("app.flow.runs", {}, RUNS);
    fake.push("app.flow.run", { run: "example-review-1" }, LIVE);
    const state = new FlowSpaceState();
    render(
      <NestProvider host={fake.host}>
        <RunsTab state={state} tabs={TABS} />
      </NestProvider>,
    );
    fireEvent.click(screen.getByTestId("flow-run-example-review-1"));
    await settle();
    expect(screen.getByTestId("flow-run-detail").dataset.run).toBe("example-review-1");
    expect(screen.getByTestId("flow-run-phase").dataset.phase).toBe("waiting");
    expect(screen.getByTestId("flow-run-phase").dataset.reason).toBe("confirm");
    expect(screen.getByTestId("flow-run-meta").dataset.tokens).toBe("1500");
    expect(screen.getByTestId("flow-node-review").dataset.tokens).toBe("1500");
    expect(screen.getByTestId("flow-node-review").dataset.elapsed).toBe("3000");
    expect(screen.getByTestId("flow-node-review").textContent).toContain("3s");

    await act(async () => fireEvent.click(screen.getByTestId("flow-pause-run")));
    expect(fake.calls).toContainEqual(["app.flow.pause", { run: "example-review-1" }]);

    expect(fake.calls).toContainEqual(["app.flow.sessions", { run: "example-review-1" }]);
    // The sub-agent the events showed is listed once (not again as Agix's task), and opens its own session.
    expect(screen.getByTestId("flow-agent-agent-1").textContent).toContain("review/review");
    expect(screen.queryByTestId("flow-task-of-main-t1")).toBeNull();
    await act(async () => fireEvent.click(screen.getByTestId("flow-agent-agent-1")));
    expect(fake.opened.at(-1)).toEqual({ instance: "system", sessionId: "agent-1" });
    await act(async () => fireEvent.click(screen.getByTestId("flow-open-main")));
    expect(fake.opened.at(-1)).toEqual({ instance: "system", sessionId: "main-1" });
    await act(async () => fireEvent.click(screen.getByTestId("flow-sidechain-side-1")));
    expect(fake.opened.at(-1)).toEqual({ instance: "system", sessionId: "side-1" });
    await act(async () => fireEvent.click(screen.getByTestId("flow-own-session-sign-sign")));
    expect(fake.opened.at(-1)).toEqual({ instance: "box", sessionId: "own-1" });
  });

  it("a paused run resumes; a dry run has its controls and no pause", async () => {
    const paused: RunView = { ...LIVE, record: { ...LIVE.record, paused: { by: { kind: "device", id: "d" }, at: 5 } }, phase: { state: "paused" } };
    const fake = fakeHost();
    fake.push("app.flow.runs", {}, [{ ...RUNS[0]!, phase: { state: "paused" } }]);
    fake.push("app.flow.dry_runs", {}, [{ id: "dry-example-review-1", flow: "example-review", status: "active", created_at: 2 }]);
    fake.push("app.flow.run", { run: "example-review-1" }, paused);
    fake.push("app.flow.run", { run: "dry-example-review-1" }, DRY);
    const state = new FlowSpaceState();
    render(
      <NestProvider host={fake.host}>
        <RunsTab state={state} tabs={TABS} />
      </NestProvider>,
    );
    fireEvent.click(screen.getByTestId("flow-run-example-review-1"));
    expect(screen.getByTestId("flow-run-phase").dataset.phase).toBe("paused");
    await act(async () => fireEvent.click(screen.getByTestId("flow-resume-run")));
    expect(fake.calls).toContainEqual(["app.flow.resume", { run: "example-review-1" }]);

    fireEvent.click(screen.getByTestId("flow-run-dry-example-review-1"));
    expect(screen.getByTestId("flow-dry-controls")).toBeTruthy();
    expect(screen.queryByTestId("flow-pause-run")).toBeNull();
    expect(screen.queryByTestId("flow-run-sessions")).toBeNull();
    fireEvent.click(screen.getByTestId("flow-node-approve"));
    expect((screen.getByTestId("flow-confirm") as HTMLButtonElement).disabled).toBe(false);
  });
});

describe("参考", () => {
  const GUIDE = "# Flow guide\n\nA flow is a **graph**.\n\n| Object | Meaning |\n|---|---|\n| **Flow** | A `name` |\n\n- one\n- two\n\n```yaml\nname: like-this\nstations: []\n```\n\n```yaml\n  - id: part\n```\n";

  it("the guide in the page's language, then the catalog and the fields; an example starts a flow like it", async () => {
    const fake = fakeHost({ "app.flow.guide": (p: { lang: string }) => ({ text: GUIDE, lang: p.lang }), "app.flow.session": conversations, "app.flow.catalog": {}, "app.flow.schema": {} });
    const state = new FlowSpaceState();
    state.attach(fake.host);
    render(
      <NestProvider host={fake.host}>
        <ReferenceTab state={state} />
      </NestProvider>,
    );
    await settle();
    expect(fake.calls).toContainEqual(["app.flow.guide", { lang: "en" }]);
    const guide = screen.getByTestId("flow-guide");
    expect(guide.dataset.lang).toBe("en");
    expect(guide.querySelector("table")?.textContent).toContain("Flow");
    expect(guide.querySelector("strong")?.textContent).toBe("graph");
    // Only a whole flow is an example to start from.
    expect(screen.getByTestId("flow-guide-example-0")).toBeTruthy();
    expect(screen.queryByTestId("flow-guide-example-1")).toBeNull();
    expect(screen.getByTestId("flow-catalog")).toBeTruthy();
    expect(screen.queryByTestId("flow-validate-box")).toBeNull();

    await act(async () => fireEvent.click(screen.getByTestId("flow-guide-like-0")));
    expect(ids(fake.opened).at(-1)).toBe("general");
    expect(fake.inserted[0]![0]).toContain("name: like-this");
  });
});

describe("a dry run's controls", () => {
  it("step and run to the end through the module, and say what was taken", async () => {
    const fake = fakeHost({
      "app.flow.dry_run.step": { run: "dry-x-1", steps: [{ kind: "task", run: "dry-x-1", station: "review", task: "review", result: "submitted" }], pending: 1, status: "active" },
      "app.flow.dry_run.run": { run: "dry-x-1", steps: [], pending: 0, status: "active" },
    });
    render(
      <NestProvider host={fake.host}>
        <DryControls module="flow" run="dry-x-1" status="active" />
      </NestProvider>,
    );
    await act(async () => fireEvent.click(screen.getByTestId("flow-dry-step")));
    expect(fake.calls).toContainEqual(["app.flow.dry_run.step", { run: "dry-x-1" }]);
    expect(screen.getByTestId("flow-dry-last").dataset.steps).toBe("1");
    expect(screen.getByTestId("flow-dry-last").textContent).toContain("review");
    await act(async () => fireEvent.click(screen.getByTestId("flow-dry-run")));
    expect(fake.calls).toContainEqual(["app.flow.dry_run.run", { run: "dry-x-1" }]);
    expect(screen.getByTestId("flow-dry-last").dataset.pending).toBe("0");
  });

  it("are off once the dry run is over", () => {
    const fake = fakeHost();
    render(
      <NestProvider host={fake.host}>
        <DryControls module="flow" run="dry-x-1" status="finished" />
      </NestProvider>,
    );
    expect((screen.getByTestId("flow-dry-step") as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByTestId("flow-dry-run") as HTMLButtonElement).disabled).toBe(true);
  });
});

describe("a station in a dry run", () => {
  it("shows the message a session task's session would have been sent", () => {
    const fake = fakeHost();
    render(
      <NestProvider host={fake.host}>
        <StationPanel station="review" def={DEF} view={DRY} />
      </NestProvider>,
    );
    expect(screen.getByTestId("flow-preview-review").textContent).toContain("flow-chat · flow_submit");
    expect(screen.getByTestId("flow-preview-message-review").textContent).toContain("## Skill `review`");
  });

  it("hands in an external task: claimed, then submitted with the token", async () => {
    const ready: RunView = {
      record: { ...DRY.record, stations: { ...DRY.record.stations, sign: { status: "working", round: 1, tasks: [{ id: "sign", status: "ready", attempt: 1 }] } } },
    };
    const fake = fakeHost({ "app.flow.claim": { token: "tok", until: 1 } });
    render(
      <NestProvider host={fake.host}>
        <StationPanel station="sign" def={DEF} view={ready} />
      </NestProvider>,
    );
    fireEvent.click(screen.getByTestId("flow-hand-in-sign"));
    fireEvent.change(screen.getByTestId("flow-hand-in-output-sign"), { target: { value: '{"signed": true}' } });
    await act(async () => fireEvent.click(screen.getByTestId("flow-hand-in-submit-sign")));
    expect(fake.calls).toEqual(
      expect.arrayContaining([
        ["app.flow.claim", { run: "dry-example-review-1", station: "sign", task: "sign" }],
        ["app.flow.submit", { run: "dry-example-review-1", station: "sign", task: "sign", token: "tok", output: { signed: true } }],
      ]),
    );
    // A session task offers no hand-in.
    cleanup();
    render(
      <NestProvider host={fake.host}>
        <StationPanel station="review" def={DEF} view={DRY} />
      </NestProvider>,
    );
    expect(screen.queryByTestId("flow-hand-in-review")).toBeNull();
  });
});

describe("a hand-in the module refuses", () => {
  const ready = (status: "ready" | "claimed"): RunView => ({
    record: { ...DRY.record, stations: { ...DRY.record.stations, sign: { status: "working", round: 1, tasks: [{ id: "sign", status, attempt: 1 }] } } },
  });

  it("keeps the claim: the error is shown, the same token hands in again, and it can be given up", async () => {
    let refuse = true;
    const fake = fakeHost({
      "app.flow.claim": { token: "tok", until: Date.now() + 60_000 },
      "app.flow.submit": () => (refuse ? Promise.reject(new Error("the output does not match the contract")) : null),
    });
    const { rerender } = render(
      <NestProvider host={fake.host}>
        <StationPanel station="sign" def={DEF} view={ready("ready")} />
      </NestProvider>,
    );
    fireEvent.click(screen.getByTestId("flow-hand-in-sign"));
    await act(async () => fireEvent.click(screen.getByTestId("flow-hand-in-submit-sign")));
    expect(screen.getByTestId("flow-hand-in-error-sign").textContent).toContain("does not match");
    // The task is now claimed (by this panel): the form stays, with the claim shown.
    rerender(
      <NestProvider host={fake.host}>
        <StationPanel station="sign" def={DEF} view={ready("claimed")} />
      </NestProvider>,
    );
    expect(screen.getByTestId("flow-hand-in-held-sign")).toBeTruthy();
    expect(screen.getByTestId("flow-hand-in-submit-sign").textContent).toBe("Hand in again");
    refuse = false;
    await act(async () => fireEvent.click(screen.getByTestId("flow-hand-in-submit-sign")));
    const claims = fake.calls.filter(([m]) => m === "app.flow.claim");
    const submits = fake.calls.filter(([m]) => m === "app.flow.submit").map(([, p]) => (p as { token: string }).token);
    expect(claims).toHaveLength(1);
    expect(submits).toEqual(["tok", "tok"]);
  });

  it("gives a held claim up as a failed attempt", async () => {
    const fake = fakeHost({
      "app.flow.claim": { token: "tok", until: Date.now() + 60_000 },
      "app.flow.submit": () => Promise.reject(new Error("no")),
    });
    const { rerender } = render(
      <NestProvider host={fake.host}>
        <StationPanel station="sign" def={DEF} view={ready("ready")} />
      </NestProvider>,
    );
    fireEvent.click(screen.getByTestId("flow-hand-in-sign"));
    await act(async () => fireEvent.click(screen.getByTestId("flow-hand-in-submit-sign")));
    rerender(
      <NestProvider host={fake.host}>
        <StationPanel station="sign" def={DEF} view={ready("claimed")} />
      </NestProvider>,
    );
    await act(async () => fireEvent.click(screen.getByTestId("flow-hand-in-give-up-sign")));
    expect(fake.calls.find(([m]) => m === "app.flow.fail")?.[1]).toMatchObject({ run: "dry-example-review-1", station: "sign", task: "sign", token: "tok" });
    // Claimed by someone else (no claim held here): nothing to hand in from this panel.
    expect(screen.queryByTestId("flow-hand-in-form-sign")).toBeNull();
    expect(screen.queryByTestId("flow-hand-in-sign")).toBeNull();
  });
});

describe("a changing tool call names what it acts on", () => {
  it("reads the target from the call's input", () => {
    expect(toolTarget("flow_update", { flow: "example-review", yaml: 'name: example-review\ntitle: "Retitled"\nstations: []' })).toEqual({
      tool: "update",
      flow: "example-review",
      title: "Retitled",
    });
    expect(toolTarget("flow_delete", '{"flow": "old"}')).toEqual({ tool: "delete", flow: "old" });
    expect(toolTarget("flow_start", { flow: "f", subject: "repo" })).toMatchObject({ tool: "start", flow: "f", subject: "repo" });
    expect(toolTarget("flow_dry_run", { flow: "f" })).toMatchObject({ tool: "dry_run", flow: "f" });
    expect(toolTarget("flow_create", { yaml: "name: authored\ntitle: Authored here\nstations: []" })).toMatchObject({ tool: "create", flow: "authored", title: "Authored here" });
    expect(toolTarget("flow_skill_set", { name: "review", body: "x" })).toMatchObject({ tool: "skill_set", skill: "review" });
    expect(toolTarget("ns__flow_skill_delete", { name: "review" })).toMatchObject({ tool: "skill_delete", skill: "review" });
    // Reading tools and anything else: no row of ours.
    expect(toolTarget("flow_show", {})).toBe(null);
    expect(toolTarget("Bash", { command: "ls" })).toBe(null);
  });

  it("is a tool.row of the space, drawing the line over the ordinary row", () => {
    const r = new Registry();
    registerFlowSpace(r);
    const block = (name: string, input: unknown) => ({ kind: "tool", id: "t", name, input, status: "running" }) as never;
    expect(r.claim("tool.row", block("flow_update", { flow: "x" }))?.id).toBe("flow.author-tool");
    expect(r.claim("tool.row", block("flow_show", {}))).toBe(null);
    const fake = fakeHost();
    render(
      <NestProvider host={fake.host}>
        <ToolTargetLine target={toolTarget("flow_update", { flow: "example-review" })!} />
        <ToolTargetLine target={toolTarget("flow_create", { yaml: "name: authored\ntitle: Authored here" })!} />
      </NestProvider>,
    );
    const [update, create] = screen.getAllByTestId("flow-tool-target");
    expect(update!.dataset.flow).toBe("example-review");
    expect(update!.textContent).toBe("Changes the flow example-review");
    expect(create!.textContent).toBe("Creates the flow authored (Authored here)");
    // A run tool names the run (xops-03 §5).
    expect(r.claim("tool.row", block("flow_pause", { run: "r-1" }))?.id).toBe("flow.author-tool");
    cleanup();
    render(
      <NestProvider host={fake.host}>
        <ToolTargetLine target={toolTarget("flow_confirm", { run: "r-1", station: "sign", pass: false })!} />
      </NestProvider>,
    );
    expect(screen.getByTestId("flow-tool-target").dataset.run).toBe("r-1");
    expect(screen.getByTestId("flow-tool-target").textContent).toBe("Rejects sign of the run r-1");
  });

  it("leaves a task session's settings entry undrawn", () => {
    const r = new Registry();
    registerFlowSpace(r);
    const entry = { kind: "custom", type: "entry", data: { id: "e", ns: "app.flow", event: "session_settings", payload: { scene: "flow-chat" }, at: 0 } } as never;
    expect(r.claim("flow.block", entry)).toBe(null);
  });
});
