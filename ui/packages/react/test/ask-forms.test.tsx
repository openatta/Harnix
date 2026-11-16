/**
 * 权限之外的三种卡：模型的提问、计划审阅、MCP server 的询问。真组件、假服务端（单元测试，testing.md §1）。
 * 定位只用 data-testid / data-id / data-state。
 */

import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { HarnixUiProvider, SessionView } from "../src";
import { FakeClient, attached, flush } from "./fake-client";

let server: FakeClient;

async function open() {
  render(
    <HarnixUiProvider client={server.asClient()}>
      <SessionView sessionId="S1" />
    </HarnixUiProvider>,
  );
  await act(async () => {
    await flush();
  });
}

async function settle() {
  await act(async () => {
    await flush();
  });
}

const card = () => screen.getByTestId("conv-ask");
const responded = () => server.called("harnix.session.respond").map((c) => c.params);

beforeEach(() => {
  server = new FakeClient();
  server.handlers["harnix.session.attach"] = (p) => attached(String(p.session_id));
});

afterEach(() => cleanup());

const question = {
  kind: "prompt",
  prompt_type: "question",
  prompt_id: "u1",
  tool_use_id: "u1",
  question: "Which branch?",
  header: "Branch",
  options: [
    { key: "a", label: "main" },
    { key: "b", label: "dev" },
  ],
};

describe("模型的提问", () => {
  it("选一个选项作答；答案走 harnix.session.respond 的 answer，卡片说答了什么", async () => {
    await open();
    act(() => server.event("S1", question));
    expect(card().dataset.kind).toBe("question");
    expect(within(card()).getByTestId("conv-ask-question").textContent).toBe("Which branch?");
    expect(card().textContent).toContain("Branch");
    const submit = within(card()).getByTestId("conv-ask-submit") as HTMLButtonElement;
    expect(submit.disabled).toBe(true);

    const options = within(card()).getAllByTestId("conv-ask-option");
    expect(options.map((o) => o.dataset.id)).toEqual(["a", "b"]);
    fireEvent.click(within(options[1]!).getByRole("radio"));
    expect(submit.disabled).toBe(false);
    fireEvent.click(submit);
    await settle();
    expect(responded()[0]).toEqual({
      instance: "main",
      session_id: "S1",
      prompt_id: "u1",
      prompt_type: "question",
      tool_use_id: "u1",
      answer: { type: "answer", text: "b" },
    });
    expect(card().dataset.state).toBe("answered");
    expect(card().dataset.answer).toBe("replied");
    expect(card().textContent).toContain("dev");
    expect(within(card()).queryAllByRole("button")).toHaveLength(0);
  });

  it("用自己的话答（Enter 提交）；没有选项的问题只有文字框；也可以不答", async () => {
    await open();
    act(() => server.event("S1", question));
    fireEvent.click(within(within(card()).getByTestId("conv-ask-other")).getByRole("radio"));
    const text = within(card()).getByTestId("conv-ask-other-text");
    fireEvent.change(text, { target: { value: "  release/1.2  " } });
    fireEvent.submit(text.closest("form")!);
    await settle();
    expect(responded()[0]!.answer).toEqual({ type: "answer", text: "release/1.2" });
    expect(card().textContent).toContain("release/1.2");
    cleanup();

    server = new FakeClient();
    server.handlers["harnix.session.attach"] = (p) => attached(String(p.session_id));
    await open();
    act(() => server.event("S1", { kind: "prompt", prompt_type: "question", prompt_id: "u2", question: "Name?", header: null, options: [] }));
    expect(within(card()).queryAllByTestId("conv-ask-option")).toHaveLength(0);
    expect(within(card()).getByTestId("conv-ask-other-text")).toBeTruthy();
    fireEvent.click(within(card()).getByTestId("conv-ask-decline"));
    await settle();
    expect(responded()[0]).toMatchObject({ prompt_type: "question", answer: { type: "decline" } });
    expect(card().dataset.answer).toBe("declined");
  });

  it("别的设备答了：卡片画出那个答案，注明在别处回答", async () => {
    await open();
    act(() => server.event("S1", question));
    act(() => server.event("S1", { kind: "ask_answered", prompt_id: "u1", prompt_type: "question", answer: { type: "answer", text: "a" } }));
    expect(card().dataset.answer).toBe("replied");
    expect(card().textContent).toContain("main");
    expect(card().textContent).toContain("别处");
    act(() => server.event("S1", { kind: "prompt_resolved", prompt_id: "u1", resolution: "answered" }));
    expect(card().dataset.answer).toBe("replied");
  });
});

describe("计划审阅", () => {
  const plan = [
    { kind: "user_message", text: "plan it" },
    { kind: "text_delta", text: "## Plan\n\n- add **a.txt**\n- run the tests" },
    { kind: "tool_use", id: "x1", name: "ExitPlanMode", input: { allowed_prompts: [{ tool: "Bash", prompt: "run tests" }] } },
    { kind: "prompt", prompt_type: "permission", prompt_id: "p1", tool_name: "ExitPlanMode", tool_use_id: "x1", message: "Exit plan mode?" },
  ];

  it("计划按 Markdown 画，带它要的放行；批准是 permit", async () => {
    await open();
    act(() => plan.forEach((e) => server.event("S1", e)));
    expect(card().dataset.kind).toBe("plan");
    const shown = within(card()).getByTestId("conv-ask-plan");
    expect(shown.querySelector("h2")?.textContent).toBe("Plan");
    expect(shown.querySelector("strong")?.textContent).toBe("a.txt");
    expect(within(card()).getByTestId("conv-ask-plan-allowed").textContent).toContain("run tests");
    fireEvent.click(within(card()).getByTestId("conv-ask-plan-approve"));
    await settle();
    expect(responded()[0]).toEqual({ instance: "main", session_id: "S1", prompt_id: "p1", tool_use_id: "x1", decision: { type: "permit" } });
    expect(card().dataset.answer).toBe("allow");
  });

  it("要求修改：写下意见，作为拒绝的理由交给模型", async () => {
    await open();
    act(() => plan.forEach((e) => server.event("S1", e)));
    fireEvent.click(within(card()).getByTestId("conv-ask-plan-changes"));
    const send = within(card()).getByTestId("conv-ask-plan-send") as HTMLButtonElement;
    expect(send.disabled).toBe(true);
    fireEvent.change(within(card()).getByTestId("conv-ask-plan-feedback"), { target: { value: "call it b.txt" } });
    fireEvent.click(send);
    await settle();
    expect(responded()[0]!.decision).toEqual({ type: "deny", reason: "call it b.txt" });
    expect(card().dataset.answer).toBe("deny");
    expect(card().textContent).toContain("call it b.txt");
  });
});

describe("MCP server 的询问", () => {
  const form = {
    kind: "prompt",
    prompt_type: "mcp_elicitation",
    prompt_id: "e1",
    server: "tracker",
    mode: "form",
    message: "Where should the issue go?",
    requested_schema: {
      type: "object",
      properties: {
        project: { type: "string", title: "Project" },
        priority: { type: "integer", minimum: 1, maximum: 3 },
        urgent: { type: "boolean" },
        kind: { type: "string", enum: ["bug", "task"], enumNames: ["Bug", "Task"] },
      },
      required: ["project", "kind"],
    },
  };

  it("按 schema 画字段，标明出处；不合的不交，改了再交", async () => {
    await open();
    act(() => server.event("S1", form));
    expect(card().dataset.kind).toBe("elicitation");
    expect(within(card()).getByTestId("conv-ask-elicit-from").textContent).toContain("tracker");
    const fields = within(card()).getAllByTestId("conv-ask-elicit-field");
    expect(fields.map((f) => f.dataset.id)).toEqual(["project", "priority", "urgent", "kind"]);

    fireEvent.change(within(fields[1]!).getByRole("textbox"), { target: { value: "9" } });
    fireEvent.click(within(card()).getByTestId("conv-ask-elicit-accept"));
    await settle();
    expect(responded()).toHaveLength(0);
    expect(within(card()).getAllByTestId("conv-ask-elicit-problem")).toHaveLength(3);

    fireEvent.change(within(fields[0]!).getByLabelText(/Project/), { target: { value: "web" } });
    fireEvent.change(within(fields[1]!).getByRole("textbox"), { target: { value: "2" } });
    fireEvent.click(within(fields[2]!).getByRole("checkbox"));
    fireEvent.change(within(fields[3]!).getByRole("combobox"), { target: { value: "bug" } });
    fireEvent.click(within(card()).getByTestId("conv-ask-elicit-accept"));
    await settle();
    expect(responded()[0]).toEqual({
      instance: "main",
      session_id: "S1",
      prompt_id: "e1",
      prompt_type: "mcp_elicitation",
      answer: { type: "accept", content: { project: "web", priority: 2, urgent: true, kind: "bug" } },
    });
    expect(card().dataset.answer).toBe("accepted");
  });

  it("server 不收（不合它的 schema）：卡片回到没答，可以再答", async () => {
    server.handlers["harnix.session.respond"] = () => {
      throw Object.assign(new Error("schema"), { code: -32602 });
    };
    await open();
    act(() => server.event("S1", form));
    fireEvent.click(within(card()).getByTestId("conv-ask-elicit-cancel"));
    await settle();
    expect(card().dataset.state).toBe("pending");
    delete server.handlers["harnix.session.respond"];
    fireEvent.click(within(card()).getByTestId("conv-ask-elicit-decline"));
    await settle();
    expect(responded().at(-1)!.answer).toEqual({ type: "decline" });
    expect(card().dataset.answer).toBe("declined");
  });

  it("网址：完整显示网址与主机，不自动打开；回来按「已完成」", async () => {
    await open();
    act(() =>
      server.event("S1", {
        kind: "prompt", prompt_type: "mcp_elicitation", prompt_id: "e2", server: "auth", mode: "url",
        message: "Sign in", url: "https://auth.example.com/login?state=x", url_host: "auth.example.com",
      }),
    );
    const url = within(card()).getByTestId("conv-ask-elicit-url");
    expect(url.textContent).toContain("https://auth.example.com/login?state=x");
    expect(url.textContent).toContain("auth.example.com");
    const link = within(card()).getByTestId("conv-ask-elicit-open") as HTMLAnchorElement;
    expect(link.getAttribute("rel")).toBe("noopener noreferrer");
    fireEvent.click(within(card()).getByTestId("conv-ask-elicit-accept"));
    await settle();
    expect(responded()[0]!.answer).toEqual({ type: "accept" });
  });
});
