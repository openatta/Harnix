/**
 * `/flow <flow> [subject]` in a conversation of a project: offered with the project's flows,
 * started at once with a subject and nothing else to ask, else the new-task dialog on that flow;
 * the task in the conversation's project, shown in its space, with a card left in it that opens
 * the task; refused with why in a conversation of no project. And "为这个项目建流程": trust first,
 * then the flows' conversation working in the project with its first message in the composer.
 */

import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { HostProvider, Registry, setLocale, type CommandCandidate, type CommandContext, type HostApi, type HostSessionRef } from "@harnix/react";

import { register } from "../src/register";
import { TaskLinkCard, asksForInput, authorForProject, conversationOf, flowArgs, registerSlashFlow, slashCandidates, taskCardOf, trustedForAuthoring } from "../src/slash";
import type { FlowSpace } from "../src/space";
import { registerFlowStrings } from "../src/strings";
import type { DefSummary } from "../src/types";

afterEach(() => cleanup());
setLocale("en");
registerFlowStrings(new Registry());

const ROOT = "/work/alpha";
const CONV: HostSessionRef = { instance: "main", sessionId: "c1" };
const DEFS: DefSummary[] = [
  { name: "says-ready", title: "Says ready", digest: "a", origin: "builtin", valid: true, inputs: { type: "object", properties: { depth: { type: "integer", default: 2 } } } },
  { name: "needs-target", title: "Needs a target", digest: "b", origin: "user", valid: true, inputs: { type: "object", properties: { target: { type: "string" } }, required: ["target"] } },
  { name: "house", title: "", digest: "", origin: "project", valid: false, reason: "untrusted", inputs: { type: "object" } },
];

/** A host with a session list, an open conversation, notices and dialogs. */
function fakeHost(answers: Record<string, unknown> = {}, rows: Record<string, unknown>[] = [{ instance: "main", session_id: "c1", space: "sessions", project_root: ROOT }]) {
  const calls: [string, unknown][] = [];
  const notices: [string, unknown][] = [];
  const dialogs: ((close: (r: unknown) => void) => unknown)[] = [];
  let front = "projects";
  const host = {
    call: vi.fn(async (method: string, params?: unknown) => {
      calls.push([method, params]);
      const answer = answers[method];
      if (answer instanceof Error) throw answer;
      return typeof answer === "function" ? (answer as (p: unknown) => unknown)(params) : (answer ?? null);
    }),
    subscribe: vi.fn(() => () => undefined),
    session: {
      current: () => CONV,
      context: () => null,
      list: () => rows,
      onChange: () => () => undefined,
      open: vi.fn(async () => undefined),
    },
    spaces: { current: () => front, switch: vi.fn(async (id: string) => void (front = id)) },
    shell: { notice: vi.fn((text: string, tone?: unknown) => void notices.push([text, tone])) },
    dialogs: {
      open: vi.fn(async (render: (close: (r: unknown) => void) => unknown) => void dialogs.push(render)),
      confirm: vi.fn(async () => true),
    },
    composer: { insert: vi.fn() },
    details: { open: vi.fn(), tab: () => "", onTab: () => () => undefined },
    sidebar: { openTab: vi.fn() },
    redraw: vi.fn(),
    t: (k: string) => k,
  } as unknown as HostApi;
  return { host, calls, notices, dialogs };
}

const ctx: CommandContext = { space: "projects", session: CONV };
const settle = () => act(async () => {});

/** The `/flow` command as registered, and its candidates for `ctx`. */
async function slash(host: HostApi, r = new Registry()) {
  registerSlashFlow(r, host, "flow", () => [], "flow");
  const command = r.at("command").find((c) => c.id === "flow.slash")!;
  const candidates = (await command.complete("", ctx)) as CommandCandidate[];
  return { command, candidates, submit: (args: string, at = ctx) => candidates[0]!.submit!(args, at) };
}

describe("/flow's words", () => {
  it("are a flow and the rest as typed; a flow asks for input only for a required one with no default", () => {
    expect(flowArgs("says-ready  fix the  login\nbug ")).toEqual({ flow: "says-ready", subject: "fix the  login\nbug" });
    expect(flowArgs("  says-ready ")).toEqual({ flow: "says-ready", subject: "" });
    expect(flowArgs("")).toEqual({ flow: "", subject: "" });
    expect(asksForInput(DEFS[0]!)).toBe(false);
    expect(asksForInput(DEFS[1]!)).toBe(true);
    expect(asksForInput({ inputs: { type: "object", properties: { on: { type: "boolean" } }, required: ["on"] } })).toBe(false);
  });

  it("offers /flow, then /flow <name> for each flow the project may start", () => {
    const submit = vi.fn();
    const list = slashCandidates(DEFS, submit);
    expect(list.map((c) => c.replacement)).toEqual(["/flow", "/flow says-ready", "/flow needs-target"]);
    expect(list[0]!.submit).toBe(submit);
    expect(list[1]!.description).toBe("Says ready");
    expect(list[1]!.submit).toBeUndefined();
  });
});

describe("the conversation /flow is typed in", () => {
  it("has the project and the space the session list says; else the front space's sessions", () => {
    const r = new Registry();
    r.register("space", { id: "projects", icon: "folder", title: () => "Projects", sessions: "sessions" }, "harnix-app");
    expect(conversationOf(fakeHost().host, r, ctx)).toEqual({ session: CONV, root: ROOT, space: "sessions" });
    const bare = fakeHost({}, [{ instance: "main", session_id: "c1" }]).host;
    expect(conversationOf(bare, r, ctx)).toEqual({ session: CONV, root: null, space: "sessions" });
    expect(conversationOf(bare, r, { space: "projects", session: null })).toBeNull();
  });
});

describe("/flow", () => {
  it("lists the flows of the conversation's project", async () => {
    const fake = fakeHost({ "flow.defs": DEFS });
    const { candidates } = await slash(fake.host);
    expect(candidates.map((c) => c.label)).toEqual(["flow", "flow says-ready", "flow needs-target"]);
    expect(fake.calls).toEqual([["flow.defs", { project: ROOT }]]);
  });

  it("with a subject and nothing to ask: starts the task in the project, shown in the conversation's space, the card left there", async () => {
    const fake = fakeHost({ "flow.defs": DEFS, "flow.start": { run: "says-ready-1" } });
    const { submit } = await slash(fake.host);
    expect(await submit("says-ready fix the login bug")).toBe(true);
    expect(fake.calls.find(([m]) => m === "flow.start")?.[1]).toEqual({
      flow: "says-ready",
      subject: "fix the login bug",
      project_root: ROOT,
      show_in_space: "sessions",
      from_session: { instance: "main", session_id: "c1", space: "sessions" },
    });
    expect(fake.dialogs).toHaveLength(0);
  });

  it("without a subject, or for a flow that asks for input: the new-task dialog on that flow, the subject filled in", async () => {
    const fake = fakeHost({ "flow.defs": DEFS, "flow.def": { def: { name: "needs-target", stations: [] }, yaml: "", digest: "b" }, "flow.start": { run: "needs-target-1" }, "flow.workspace_info": { git: false, reason: "not_a_repository" } });
    const { submit } = await slash(fake.host);
    expect(await submit("needs-target the api")).toBe(true);
    expect(fake.calls.some(([m]) => m === "flow.start")).toBe(false);
    const close = vi.fn();
    const page = render(<HostProvider host={fake.host}>{fake.dialogs[0]!(close) as never}</HostProvider>);
    await settle();
    expect(page.getByTestId("task-flow-needs-target").getAttribute("aria-pressed")).toBe("true");
    expect((page.getByTestId("task-subject") as HTMLTextAreaElement).value).toBe("the api");
    fireEvent.change(page.getByTestId("task-input-target").querySelector("input")!, { target: { value: "src/" } });
    fireEvent.click(page.getByTestId("task-start"));
    await settle();
    expect(fake.calls.find(([m]) => m === "flow.start")?.[1]).toMatchObject({ flow: "needs-target", subject: "the api", project_root: ROOT, show_in_space: "sessions", from_session: { session_id: "c1", space: "sessions" } });
    // The conversation stays in front: the task is the card's to open.
    expect(fake.host.session.open).not.toHaveBeenCalled();
    cleanup();

    expect(await submit("says-ready")).toBe(true);
    expect(fake.dialogs).toHaveLength(2);
    expect(await submit("")).toBe(true);
    expect(fake.dialogs).toHaveLength(3);
  });

  it("refuses with why: no project, no such flow, one that cannot start; the text stays", async () => {
    const fake = fakeHost({ "flow.defs": DEFS });
    const { submit } = await slash(fake.host);
    expect(await submit("nope x")).toBe(false);
    expect(await submit("house x")).toBe(false);
    expect(fake.notices.map(([text]) => text)).toEqual(["No flow called nope here.", expect.stringContaining("house cannot be started")]);
    const none = fakeHost({ "flow.defs": DEFS }, [{ instance: "main", session_id: "c1", space: "sessions" }]);
    const bare = await slash(none.host);
    expect(bare.candidates.map((c) => c.label)).toEqual(["flow"]);
    expect(await bare.submit("says-ready x")).toBe(false);
    expect(none.notices[0]![0]).toContain("A task belongs to a project");
    expect(none.calls.some(([m]) => m === "flow.start")).toBe(false);
    expect(await bare.submit("says-ready x", { space: "projects", session: null })).toBe(false);
  });
});

describe("the card", () => {
  const entry = (payload: unknown, ns = "flow", event = "task_started") => ({ kind: "custom", type: "entry", data: { id: "e1", ns, event, payload, at: 3 } });

  it("is the module's task_started entry", () => {
    expect(taskCardOf(entry({ run: "says-ready-1", flow: "says-ready", flow_title: "Says ready", title: "", subject: "fix it", project_root: ROOT }))).toEqual({
      id: "e1",
      run: "says-ready-1",
      flow: "says-ready",
      flowTitle: "Says ready",
      title: "",
      subject: "fix it",
      projectRoot: ROOT,
    });
    expect(taskCardOf(entry({ run: "x" }, "other"))).toBeNull();
    expect(taskCardOf(entry({ run: "x" }, "flow", "proposal"))).toBeNull();
    expect(taskCardOf(entry({}))).toBeNull();
  });

  it("names the flow and the subject, and opens the task", async () => {
    const fake = fakeHost();
    const open = vi.fn();
    const card = taskCardOf(entry({ run: "says-ready-1", flow: "says-ready", flow_title: "Says ready", subject: "fix it" }))!;
    const page = render(
      <HostProvider host={fake.host}>
        <TaskLinkCard card={card} module="flow" open={open} />
      </HostProvider>,
    );
    await settle();
    expect(page.getByTestId("flow-task-card").dataset.run).toBe("says-ready-1");
    expect(page.getByTestId("flow-task-card").textContent).toContain("Task started with Says ready");
    expect(page.getByTestId("flow-task-card").textContent).toContain("fix it");
    fireEvent.click(page.getByTestId("flow-task-card-open"));
    expect(open).toHaveBeenCalledTimes(1);
  });

  it("is registered with the command by the package", () => {
    const fake = fakeHost();
    const r = new Registry();
    r.activate("flow", () => register(r, fake.host));
    r.settle();
    expect(r.at("command").map((c) => c.id)).toContain("flow.slash");
    const block = r.at("flow.block").find((c) => c.id === "flow.task-card")!;
    expect(block.match(entry({ run: "says-ready-1" }) as never)).toBe(true);
    expect(r.refusals()).toEqual([]);
  });
});

describe("为这个项目建流程", () => {
  const space = (opened: HostSessionRef | null) =>
    ({ module: "flow", space: "flow", state: { enter: vi.fn(async () => undefined), openGeneral: vi.fn(async () => opened) } }) as unknown as FlowSpace;

  it("a trusted project: the flows' conversation working in it, its first message in the composer", async () => {
    const fake = fakeHost({ "flow.focus": { flow: null, project: ROOT } });
    const view = space({ instance: "main", sessionId: "general" });
    const r = new Registry();
    r.register("space", { id: "projects", icon: "folder", title: () => "Projects" }, "harnix-app");
    await authorForProject(fake.host, r, view, { root: ROOT, trust: { status: async () => "trusted", ask: vi.fn() } });
    expect(view.state.enter).toHaveBeenCalledWith(null, null, { from: { space: "projects", title: "Projects" } });
    expect(fake.calls).toEqual([["flow.focus", { instance: "main", session_id: "general", flow: null, project: ROOT }]]);
    expect(fake.host.composer.insert).toHaveBeenCalledWith(expect.stringContaining(ROOT), { session: { instance: "main", sessionId: "general" } });
  });

  it("an untrusted one is offered trust first; declined, nothing opens", async () => {
    const fake = fakeHost();
    const ask = vi.fn(async () => true);
    expect(await trustedForAuthoring(fake.host, { root: ROOT, trust: { status: async () => "untrusted", ask } })).toBe(true);
    expect(fake.host.dialogs.confirm).toHaveBeenCalledWith(expect.stringContaining(`${ROOT}/.harnix/flows/`), expect.anything());
    expect(ask).toHaveBeenCalledTimes(1);
    vi.mocked(fake.host.dialogs.confirm).mockResolvedValueOnce(false);
    const view = space({ instance: "main", sessionId: "general" });
    await authorForProject(fake.host, new Registry(), view, { root: ROOT, trust: { status: async () => "untrusted", ask } });
    expect(ask).toHaveBeenCalledTimes(1);
    expect(view.state.enter).not.toHaveBeenCalled();
    // Nothing to trust: said, and nothing opens.
    expect(await trustedForAuthoring(fake.host, { root: ROOT, trust: { status: async () => "none", ask } })).toBe(false);
    expect(fake.notices.at(-1)![0]).toContain("cannot be trusted");
    // Trust that cannot be read: the module has the last word on a save.
    expect(await trustedForAuthoring(fake.host, { root: ROOT, trust: { status: async () => null, ask } })).toBe(true);
    expect(await trustedForAuthoring(fake.host, { root: ROOT })).toBe(true);
  });

  it("is in the project's menu by the task kind", () => {
    const fake = fakeHost();
    const r = new Registry();
    r.activate("harnix-app", () => r.definePoint("harnix-app.project.items", { kind: "register" }));
    r.activate("flow", () => register(r, fake.host));
    r.settle();
    const kind = r.list<{ id: string; projectMenu?(c: unknown): { id: string; label: string }[] }>("harnix-app.project.items").find((k) => k.id === "flow.task")!;
    expect(kind.projectMenu!({ root: ROOT, space: "sessions" }).map((i) => [i.id, i.label])).toEqual([["flow.author-for-project", "Create a flow for this project"]]);
  });
});
