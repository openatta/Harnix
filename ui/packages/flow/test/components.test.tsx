/**
 * The components against a fake host: the graph in both modes, the station panel's buttons per
 * status and the calls they make, the proposal card, and the space's registration.
 */

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { NestProvider, Registry, setLocale, type NestHost } from "@harnix/react";

import { FlowGraph } from "../src/FlowGraph";
import { ProposalCard } from "../src/ProposalCard";
import { registerFlowSpace } from "../src/space";
import { StationPanel } from "../src/StationPanel";
import { STRINGS, registerFlowStrings } from "../src/strings";
import type { FlowDef, RunView, StationRec } from "../src/types";

afterEach(cleanup);
setLocale("en");
registerFlowStrings(new Registry());

const DEF: FlowDef = {
  name: "demo-review",
  title: "Demo review",
  stations: [
    { id: "collect", executor: { kind: "program", step: "collect" }, output: { type: "object" }, exit: [{ kind: "output" }] },
    {
      id: "analyze",
      foreach: "${stations.collect.output.modules}",
      executor: { kind: "session", scene: "flow-chat" },
      skill: "analyze",
      output: { type: "object", required: ["findings"] },
      exit: [{ kind: "output" }],
      hooks: [{ on: "station.passed", action: "notify" }],
    },
    { id: "lint", needs: ["collect"], exit: [{ kind: "gate", gate: "lint" }] },
    { id: "review", needs: ["analyze", "lint"], exit: [{ kind: "confirm", who: "reviewer" }], rework_to: ["analyze", "lint"] },
  ],
};

function view(stations: Record<string, StationRec>, status: "active" | "finished" = "active"): RunView {
  return {
    record: {
      id: "r1",
      flow: "demo-review",
      def_digest: "d",
      subject: "demo",
      status,
      rev: 3,
      created_at: 0,
      created_by: { kind: "device", id: "x" },
      stations,
    },
    def: DEF,
    unmet: { review: [{ kind: "confirm" }] },
  };
}

const RUNNING = view({
  collect: { status: "passed", round: 1, tasks: [{ id: "collect", status: "submitted", attempt: 1, output: { modules: ["a", "b"] } }] },
  analyze: {
    status: "passed",
    round: 2,
    tasks: [
      { id: "0", status: "submitted", attempt: 1, item: "a", session: { instance: "flow", session_id: "s-a" } },
      { id: "1", status: "submitted", attempt: 1, item: "b" },
    ],
    forced: { by: { kind: "device", id: "x" }, at: 0, reason: "ok" },
  },
  lint: { status: "passed", round: 1, tasks: [] },
  review: { status: "working", round: 1, tasks: [] },
});

function fakeHost(answers: Record<string, unknown> = {}) {
  const calls: [string, unknown][] = [];
  const opened: unknown[] = [];
  const host = {
    call: vi.fn(async (method: string, params?: unknown) => {
      calls.push([method, params]);
      return answers[method] ?? null;
    }),
    subscribe: vi.fn(() => () => {}),
    session: { open: vi.fn((ref: unknown) => void opened.push(ref)) },
    details: { open: vi.fn() },
    t: (k: string) => k,
  } as unknown as NestHost;
  return { host, calls, opened };
}

describe("FlowGraph", () => {
  it("draws the definition: one button per station, needs and rework edges, foreach, hooks", () => {
    const { host } = fakeHost();
    const picked: string[] = [];
    render(
      <NestProvider host={host}>
        <FlowGraph def={DEF} onSelect={(s) => picked.push(s)} />
      </NestProvider>,
    );
    expect(screen.getByTestId("flow-graph").dataset.mode).toBe("definition");
    for (const s of DEF.stations) expect(screen.getByTestId(`flow-node-${s.id}`).getAttribute("role")).toBe("button");
    expect(screen.getByTestId("flow-node-analyze").getAttribute("aria-label")).toContain("session ×N");
    expect(screen.getByTestId("flow-node-analyze").textContent).toContain("analyze");
    expect(screen.getByTestId("flow-hook-analyze")).toBeTruthy();
    expect(screen.getByTestId("flow-node-lint").textContent).toContain("gate");
    expect(document.querySelectorAll(".hx-flow-back")).toHaveLength(2);
    expect(screen.getByTestId("flow-node-review").dataset.status).toBeUndefined();
    fireEvent.click(screen.getByTestId("flow-node-review"));
    fireEvent.keyDown(screen.getByTestId("flow-node-lint"), { key: "Enter" });
    expect(picked).toEqual(["review", "lint"]);
  });

  it("draws a run: status, round, progress, forced", () => {
    const { host } = fakeHost();
    render(
      <NestProvider host={host}>
        <FlowGraph def={DEF} view={RUNNING} selected="review" />
      </NestProvider>,
    );
    expect(screen.getByTestId("flow-graph").dataset.mode).toBe("run");
    expect(screen.getByTestId("flow-node-review").dataset.status).toBe("working");
    expect(screen.getByTestId("flow-node-review").getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByTestId("flow-node-analyze").dataset.status).toBe("passed");
    expect(screen.getByTestId("flow-node-analyze").textContent).toContain("2/2");
    expect(screen.getByTestId("flow-node-analyze").getAttribute("aria-label")).toContain("round 2");
    expect(screen.getByTestId("flow-forced-analyze")).toBeTruthy();
    expect(screen.getByTestId("flow-node-review").getAttribute("class")).toContain("hx-flow-tone-info");
  });
});

describe("StationPanel", () => {
  it("enables what applies to a working confirm station and calls the module's methods", async () => {
    const { host, calls } = fakeHost();
    render(
      <NestProvider host={host}>
        <StationPanel station="review" def={DEF} view={RUNNING} module="flow" />
      </NestProvider>,
    );
    const btn = (id: string) => screen.getByTestId(id) as HTMLButtonElement;
    expect(btn("flow-confirm").disabled).toBe(false);
    expect(btn("flow-force").disabled).toBe(false);
    expect(btn("flow-rework").disabled).toBe(false);
    expect(btn("flow-release").disabled).toBe(true);

    await act(async () => fireEvent.click(btn("flow-confirm")));
    expect(calls).toContainEqual(["app.flow.confirm", { run: "r1", station: "review", pass: true }]);

    // Force needs a reason.
    fireEvent.click(btn("flow-force"));
    expect(btn("flow-action-submit").disabled).toBe(true);
    fireEvent.change(screen.getByTestId("flow-reason"), { target: { value: "deadline" } });
    await act(async () => fireEvent.click(btn("flow-action-submit")));
    expect(calls).toContainEqual(["app.flow.force", { run: "r1", station: "review", reason: "deadline" }]);

    // Rework to a chosen target, with some of its tasks.
    fireEvent.click(btn("flow-rework"));
    expect((screen.getByTestId("flow-rework-target") as HTMLSelectElement).value).toBe("analyze");
    fireEvent.click(screen.getByTestId("flow-rework-task-1"));
    fireEvent.change(screen.getByTestId("flow-reason"), { target: { value: "thin" } });
    await act(async () => fireEvent.click(btn("flow-action-submit")));
    expect(calls).toContainEqual(["app.flow.rework", { run: "r1", to: "analyze", tasks: ["1"], reason: "thin" }]);
  });

  it("disables everything on a passed station without rework targets, offers retry per failed task, opens sessions", async () => {
    const failing = view({
      ...RUNNING.record.stations,
      analyze: {
        status: "working",
        round: 1,
        tasks: [
          { id: "0", status: "failed", attempt: 1, error: "boom", session: { instance: "flow", session_id: "s-a" } },
          { id: "1", status: "submitted", attempt: 1 },
        ],
      },
    });
    const { host, calls, opened } = fakeHost();
    render(
      <NestProvider host={host}>
        <StationPanel station="collect" def={DEF} view={failing} module="ops" />
      </NestProvider>,
    );
    for (const id of ["flow-confirm", "flow-reject", "flow-force", "flow-rework", "flow-release"]) {
      expect((screen.getByTestId(id) as HTMLButtonElement).disabled, id).toBe(true);
    }
    cleanup();
    render(
      <NestProvider host={host}>
        <StationPanel station="analyze" def={DEF} view={failing} module="ops" />
      </NestProvider>,
    );
    expect(screen.queryByTestId("flow-retry-1")).toBeNull();
    await act(async () => fireEvent.click(screen.getByTestId("flow-retry-0")));
    expect(calls).toContainEqual(["app.ops.retry", { run: "r1", station: "analyze", task: "0" }]);
    fireEvent.click(screen.getByTestId("flow-open-session-0"));
    expect(opened).toEqual([{ instance: "flow", sessionId: "s-a" }]);
  });

  it("shows the definition tab with the skill body and the output contract", () => {
    const { host } = fakeHost();
    render(
      <NestProvider host={host}>
        <StationPanel station="analyze" def={DEF} info={{ def: DEF, yaml: "", digest: "d", skills: { analyze: "Read the module." } }} />
      </NestProvider>,
    );
    expect(screen.getByTestId("flow-definition")).toBeTruthy();
    expect(screen.getByTestId("flow-skill-analyze").textContent).toContain("Read the module.");
    expect(screen.getByTestId("flow-output-contract").textContent).toContain("findings");
    expect((screen.getByTestId("flow-tab-run") as HTMLButtonElement).disabled).toBe(true);
  });
});

describe("ProposalCard", () => {
  it("accepts a rework through the module and dismisses", async () => {
    const { host, calls } = fakeHost();
    render(
      <NestProvider host={host}>
        <ProposalCard id="e1" proposal={{ run: "r1", station: "review", kind: "rework", to: "analyze", reason: "thin" }} />
      </NestProvider>,
    );
    await act(async () => fireEvent.click(screen.getByTestId("flow-proposal-accept")));
    expect(calls).toEqual([["app.flow.rework", { run: "r1", to: "analyze", reason: "thin" }]]);
    expect(screen.getByTestId("flow-proposal-accepted")).toBeTruthy();
    cleanup();
    render(
      <NestProvider host={host}>
        <ProposalCard id="e2" proposal={{ run: "r1", station: "review", kind: "cancel", reason: "wrong subject" }} />
      </NestProvider>,
    );
    act(() => fireEvent.click(screen.getByTestId("flow-proposal-dismiss")));
    expect(screen.queryByTestId("flow-proposal")).toBeNull();
  });
});

describe("registerFlowSpace", () => {
  it("registers the space, the proposal block and the strings, with nothing refused", () => {
    const r = new Registry();
    const flow = registerFlowSpace(r);
    expect(r.refusals()).toEqual([]);
    const space = r.at("space")[0]!;
    expect(space).toMatchObject({ id: "flow", center: "hidden", newSession: false, owner: "flowui" });
    expect(typeof space.sidebar).toBe("function");
    expect(typeof space.overview).toBe("function");
    expect((space.tabs as { id: string }[]).map((x) => x.id)).toEqual([flow.tabs.station, flow.tabs.reference]);
    const block = { kind: "custom", type: "entry", data: { id: "e", ns: "app.flow", event: "proposal", payload: { run: "r", station: "s", kind: "cancel", reason: "x" }, at: 0 } } as never;
    expect(r.claim("flow.block", block)?.id).toBe("flow.proposal");
  });

  it("follows a module registered under another id", () => {
    const r = new Registry();
    registerFlowSpace(r, { space: "ops", module: "ops" });
    const block = (ns: string) => ({ kind: "custom", type: "entry", data: { id: "e", ns, event: "proposal", payload: { run: "r", station: "s", kind: "cancel", reason: "x" }, at: 0 } }) as never;
    expect(r.claim("flow.block", block("app.ops"))?.id).toBe("ops.proposal");
    expect(r.claim("flow.block", block("app.flow"))).toBe(null);
  });
});

describe("strings in use", () => {
  it("every literal flowui key in the sources is in the dictionary", () => {
    const dir = join(process.cwd(), "src");
    const used = new Set<string>();
    for (const f of readdirSync(dir).filter((x) => /\.tsx?$/.test(x))) {
      for (const [, key] of readFileSync(join(dir, f), "utf8").matchAll(/"(flowui\.[A-Za-z0-9_.]*[A-Za-z0-9_])"/g)) used.add(key!);
    }
    expect([...used].filter((k) => !(k in STRINGS.en))).toEqual([]);
  });
});
