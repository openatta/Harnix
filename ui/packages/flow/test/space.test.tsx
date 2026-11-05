/**
 * The flow space of xops-02 §7 against a fake host: the selection is the conversation's focus (sent
 * on select and on a session change, followed from the topic), the line over the composer, the
 * right column's tabs, a dry run's controls, and the message a dry run's session task would have
 * been sent.
 */

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { NestProvider, Registry, setLocale, type HostSessionRef, type NestHost } from "@harnix/react";

import { registerFlowSpace } from "../src/space";
import { FlowSpaceState } from "../src/state";
import { StationPanel } from "../src/StationPanel";
import { registerFlowStrings } from "../src/strings";
import { DryControls, FlowsTab, FocusNotice, RunsTab, type FlowTabs } from "../src/tabs";
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

/** A host whose topics can be pushed to, with an open session and a front space. */
function fakeHost(answers: Record<string, unknown> = {}) {
  const calls: [string, unknown][] = [];
  const topics = new Map<string, Set<(v: unknown) => void>>();
  const values = new Map<string, unknown>();
  const changes = new Set<(s: HostSessionRef | null) => void>();
  let current: HostSessionRef | null = { instance: "system", sessionId: "s1" };
  let front = "flow";
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
      open: vi.fn(),
    },
    spaces: { current: () => front },
    details: { open: vi.fn() },
    redraw: vi.fn(),
    t: (k: string) => k,
  } as unknown as NestHost;
  return {
    host,
    calls,
    push(topic: string, params: Record<string, unknown> | undefined, value: unknown) {
      const key = keyOf(topic, params);
      values.set(key, value);
      for (const fn of [...(topics.get(key) ?? [])]) fn(value);
    },
    open(session: HostSessionRef | null, space = "flow") {
      current = session;
      front = space;
      for (const fn of [...changes]) fn(session);
    },
  };
}

const focusCalls = (calls: [string, unknown][]) => calls.filter(([m]) => m === "app.flow.focus").map(([, p]) => p);
const settle = () => act(async () => {});

describe("the selection is the conversation's focus", () => {
  it("is sent for the open session on select, and again when another session of the space opens", async () => {
    const fake = fakeHost({ "app.flow.focus": (p: { flow: string | null }) => ({ flow: p.flow }) });
    const state = new FlowSpaceState();
    state.attach(fake.host);
    await settle();
    // Nothing selected yet: nothing is sent, the session keeps its own.
    expect(focusCalls(fake.calls)).toEqual([]);

    state.selectFlow("example-review");
    await settle();
    expect(focusCalls(fake.calls)).toEqual([{ instance: "system", session_id: "s1", flow: "example-review" }]);
    expect(state.focusOf()).toBe("example-review");

    act(() => fake.open({ instance: "system", sessionId: "s2" }));
    await settle();
    expect(focusCalls(fake.calls).at(-1)).toEqual({ instance: "system", session_id: "s2", flow: "example-review" });

    // A session of another space is not ours: nothing is sent for it.
    const before = fake.calls.length;
    act(() => fake.open({ instance: "system", sessionId: "other" }, "sessions"));
    await settle();
    expect(fake.calls.length).toBe(before);
    state.selectFlow("broken");
    await settle();
    expect(focusCalls(fake.calls).at(-1)).toEqual({ instance: "system", session_id: "s2", flow: "example-review" });
  });

  it("follows the topic: a flow the Agent created is selected, a deleted one clears the selection", async () => {
    const fake = fakeHost({ "app.flow.focus": (p: { flow: string | null }) => ({ flow: p.flow }) });
    const state = new FlowSpaceState();
    state.attach(fake.host);
    const topic = { instance: "system", session_id: "s1" };
    act(() => fake.push("app.flow.focus", topic, { flow: null }));
    expect(state.get().flow).toBe(null);
    act(() => fake.push("app.flow.focus", topic, { flow: "authored" }));
    expect(state.get()).toMatchObject({ flow: "authored", run: null });
    act(() => fake.push("app.flow.focus", topic, { flow: null }));
    expect(state.get().flow).toBe(null);
    // Following sends nothing back.
    expect(focusCalls(fake.calls)).toEqual([]);
  });

  it("takes up a session's remembered flow when nothing is selected", async () => {
    const fake = fakeHost();
    fake.push("app.flow.focus", { instance: "system", session_id: "s1" }, { flow: "example-review" });
    const state = new FlowSpaceState();
    state.attach(fake.host);
    expect(state.get().flow).toBe("example-review");
    expect(focusCalls(fake.calls)).toEqual([]);
  });

  it("does not follow the old value while its own call is unanswered", async () => {
    let answer: (v: unknown) => void = () => {};
    const fake = fakeHost({ "app.flow.focus": () => new Promise((resolve) => (answer = resolve)) });
    const state = new FlowSpaceState();
    state.attach(fake.host);
    state.selectFlow("example-review");
    act(() => fake.push("app.flow.focus", { instance: "system", session_id: "s1" }, { flow: null }));
    expect(state.get().flow).toBe("example-review");
    await act(async () => answer({ flow: "example-review" }));
    expect(state.get().flow).toBe("example-review");
  });
});

describe("the line over the composer", () => {
  it("names the current flow, or says to select one first — and that creating needs none", async () => {
    const fake = fakeHost({ "app.flow.focus": (p: { flow: string | null }) => ({ flow: p.flow }) });
    fake.push("app.flow.defs", {}, DEFS);
    const state = new FlowSpaceState();
    render(
      <NestProvider host={fake.host}>
        <FocusNotice state={state} session={{ instance: "system", sessionId: "s1" }} />
      </NestProvider>,
    );
    const notice = screen.getByTestId("flow-focus-notice");
    expect(notice.dataset.flow).toBe("");
    expect(notice.textContent).toContain("select one on the right");
    expect(notice.textContent).toContain("Creating a new flow needs no selection");
    await act(async () => state.selectFlow("example-review"));
    expect(screen.getByTestId("flow-focus-notice").dataset.flow).toBe("example-review");
    expect(screen.getByTestId("flow-focus-notice").textContent).toContain("Current flow: Example: review and approve");
  });

  it("is what the space gives the shell, and the new conversation's line follows the selection", () => {
    const r = new Registry();
    const flow = registerFlowSpace(r);
    const space = r.at("space")[0]!;
    expect(space.notice?.()).toContain("No flow selected");
    flow.state.selectFlow("example-review");
    expect(space.notice?.()).toContain("example-review");
  });
});

const TABS: FlowTabs = { flows: "flow.flows", runs: "flow.runs", reference: "flow.reference" };

describe("the right column", () => {
  it("流程: the flows with their origin and validity; the selected one's graph, skills and dry runs", async () => {
    const fake = fakeHost({
      "app.flow.def": INFO,
      "app.flow.focus": (p: { flow: string | null }) => ({ flow: p.flow }),
      "app.flow.dry_run.start": { run: "dry-example-review-2" },
    });
    fake.push("app.flow.defs", {}, DEFS);
    fake.push("app.flow.dry_runs", {}, [
      { id: "dry-example-review-1", flow: "example-review", status: "active", created_at: 1 },
      { id: "dry-other-1", flow: "other", status: "finished", created_at: 1 },
    ]);
    const state = new FlowSpaceState();
    render(
      <NestProvider host={fake.host}>
        <FlowsTab state={state} tabs={TABS} />
      </NestProvider>,
    );
    expect(screen.getByTestId("flow-def-example-review").dataset.origin).toBe("builtin");
    expect(screen.getByTestId("flow-def-example-review").textContent).toContain("built in");
    expect(screen.getByTestId("flow-def-broken").dataset.valid).toBe("false");
    expect(screen.getByTestId("flow-def-broken").textContent).toContain("invalid");
    expect(screen.getByTestId("flow-pick-hint")).toBeTruthy();

    await act(async () => fireEvent.click(screen.getByTestId("flow-def-example-review")));
    expect(focusCalls(fake.calls)).toEqual([{ instance: "system", session_id: "s1", flow: "example-review" }]);
    expect(screen.getByTestId("flow-detail").dataset.digest).toBe("aaaa1111");
    expect(screen.getByTestId("flow-graph").dataset.mode).toBe("definition");
    expect(screen.getByTestId("flow-skill-body-review").textContent).toContain("Look at the subject.");
    // Only this flow's dry runs.
    expect(screen.getByTestId("flow-dry-dry-example-review-1")).toBeTruthy();
    expect(screen.queryByTestId("flow-dry-dry-other-1")).toBeNull();
    // Nothing here edits a definition.
    expect(document.querySelector("textarea")).toBeNull();
    expect(fake.calls.some(([m]) => /app\.flow\.(save|delete|skill)/.test(m))).toBe(false);

    // A station of the graph: its definition below.
    fireEvent.click(screen.getByTestId("flow-node-review"));
    expect(screen.getByTestId("flow-station-panel").dataset.station).toBe("review");

    // A dry run, started here, is shown in the runs tab.
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

  it("运行: the selected flow's runs and dry runs; a run's graph and its station's details", async () => {
    const fake = fakeHost();
    fake.push("app.flow.defs", {}, DEFS);
    fake.push("app.flow.runs", {}, [
      { id: "example-review-1", flow: "example-review", subject: "repo", status: "active", created_at: 3 },
      { id: "other-1", flow: "other", subject: "x", status: "finished", created_at: 4 },
    ]);
    fake.push("app.flow.dry_runs", {}, [{ id: "dry-example-review-1", flow: "example-review", status: "active", created_at: 2 }]);
    fake.push("app.flow.run", { run: "dry-example-review-1" }, DRY);
    const state = new FlowSpaceState();
    state.selectFlow("example-review");
    render(
      <NestProvider host={fake.host}>
        <RunsTab state={state} tabs={TABS} />
      </NestProvider>,
    );
    const rows = [...screen.getByTestId("flow-run-list").querySelectorAll("button")].map((b) => b.dataset.testid);
    expect(rows).toEqual(["flow-run-example-review-1", "flow-run-dry-example-review-1"]);
    expect(screen.getByTestId("flow-run-dry-example-review-1").dataset.dry).toBe("true");

    fireEvent.click(screen.getByTestId("flow-run-dry-example-review-1"));
    expect(screen.getByTestId("flow-run-detail").dataset.run).toBe("dry-example-review-1");
    expect(screen.getByTestId("flow-graph").dataset.mode).toBe("run");
    expect(screen.getByTestId("flow-run-status").dataset.status).toBe("active");
    expect(screen.getByTestId("flow-dry-controls")).toBeTruthy();
    fireEvent.click(screen.getByTestId("flow-node-approve"));
    expect(screen.getByTestId("flow-station-panel").dataset.station).toBe("approve");
    expect((screen.getByTestId("flow-confirm") as HTMLButtonElement).disabled).toBe(false);

    // A real run has no dry-run controls.
    fireEvent.click(screen.getByTestId("flow-run-example-review-1"));
    expect(screen.queryByTestId("flow-dry-controls")).toBeNull();
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
