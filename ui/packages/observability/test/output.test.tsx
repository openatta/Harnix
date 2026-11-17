/**
 * The output tab: its address names the session and the task, the tool row's action finds the task
 * its call started, and the tab follows that task's output without colour codes.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import { Registry, RegistryProvider, TabInfoProvider, sidebarTypeOfKind, type ActionTarget, type TabInfo } from "@harnix/react";

import { outputAddress, parseOutputAddress, register, stripAnsi } from "../src/index";
import { scripted } from "./host";

afterEach(cleanup);

const tick = () => new Promise((r) => setTimeout(r, 0));
const session = { instance: "main", sessionId: "s/1" };
const monitor = { task_id: "t-1", kind: "monitor", status: "running", detail: { command: "count.sh", description: "Count to ten", tool_use_id: "tu-9" } };

function registered(answer: (method: string) => unknown = () => ({})) {
  const s = scripted(answer);
  const opened: { address: string; options: unknown }[] = [];
  const notices: string[] = [];
  Object.assign(s.host, {
    instances: { tasks: async () => ({ tasks: [monitor, { task_id: "t-2", kind: "agent", status: "done", detail: { tool_use_id: "tu-3" } }] }) },
    sidebar: { openResource: (address: string, options: unknown) => (opened.push({ address, options }), "tab-1") },
    shell: { notice: (text: string) => void notices.push(text) },
    session: { current: () => null, isOpen: () => false },
  });
  const r = new Registry();
  r.register("space", { id: "sessions", icon: "chat", title: () => "Sessions" }, "test");
  r.activate("observability", () => register(r, s.host));
  r.settle();
  return { s, r, opened, notices };
}

describe("an output tab's address", () => {
  it("round-trips the session and the task, whatever characters they hold", () => {
    expect(parseOutputAddress(outputAddress(session, "t 1"))).toEqual({ session, taskId: "t 1" });
  });
  it("names nothing when it is not an output address", () => {
    expect(parseOutputAddress("harnix-resource://file/a.ts")).toBeNull();
    expect(parseOutputAddress("harnix-resource://output/only/two")).toBeNull();
    expect(parseOutputAddress(null)).toBeNull();
  });
});

describe("colour codes", () => {
  it("are taken out of the text and nothing else is", () => {
    expect(stripAnsi("\u001b[31mred\u001b[0m plain \u001b]0;title\u0007end")).toBe("red plain end");
    expect(stripAnsi("no codes")).toBe("no codes");
  });
});

describe("the output tab type", () => {
  it("is one tab per task and keeps its body mounted", () => {
    const { r } = registered();
    const type = sidebarTypeOfKind(r.at("sidebar.tab"), "output")!;
    expect(type.multiple).toBe(true);
    expect(type.keepMounted).toBe(true);
    expect(type.title(outputAddress(session, "t-1"))).toBe("Output");
  });

  it("is offered on the rows of the tools that start a task, and opens the task the call started", async () => {
    const { r, opened, notices } = registered();
    const action = r.at("session.action").find((a) => a.id === "observability.output")!;
    const tool = (name: string, id: string): ActionTarget => ({ kind: "tool", session, space: "sessions", toolUseId: id, tool: name });
    expect(action.match(tool("Monitor", "tu-9"))).toBe(true);
    expect(action.match(tool("Read", "tu-9"))).toBe(false);
    await action.run(tool("Monitor", "tu-9"));
    expect(opened).toEqual([{ address: outputAddress(session, "t-1"), options: { kind: "output", params: { task: "t-1" } } }]);
    // Named by its words from then on.
    expect(sidebarTypeOfKind(r.at("sidebar.tab"), "output")!.title(opened[0]!.address)).toBe("Count to ten");
    // A call whose task is not a stream (or none) says so and opens nothing.
    await action.run(tool("Bash", "tu-3"));
    expect(opened).toHaveLength(1);
    expect(notices).toEqual(["This call has no background task to show output for."]);
  });

  it("shows the task, follows its output, and shows how it ended", async () => {
    const { s, r } = registered((method) => (method === "observability.tasks.output" ? { output: "\u001b[32mhello\u001b[0m\n", from_byte: 0, next_byte: 14, dropped: 0, final: false } : {}));
    const type = sidebarTypeOfKind(r.at("sidebar.tab"), "output")!;
    const info = { tab: { id: "t", kind: "output", address: outputAddress(session, "t-1"), visible: true, navigation: { params: {}, revision: 1 }, signal: new AbortController().signal, actions: {} } } as unknown as TabInfo;
    render(
      <RegistryProvider registry={r}>
        <TabInfoProvider value={info}>{type.render()}</TabInfoProvider>
      </RegistryProvider>,
    );
    await act(async () => {
      s.push("harnix.session.tasks", { tasks: [monitor] });
      await tick();
    });
    expect(screen.getByTestId("output-tab").getAttribute("data-status")).toBe("running");
    expect(screen.getByTestId("task-output-text").textContent).toBe("hello\n");
    // Every read of the output claims the front space's.
    expect(s.calls.find((c) => c.method === "observability.tasks.output")?.params).toEqual({ instance: "main", session_id: "s/1", space: "sessions", task_id: "t-1" });
    await act(async () => {
      s.push("harnix.session.tasks", { tasks: [{ ...monitor, status: "failed", error: "exit 2" }] });
      await tick();
    });
    expect(screen.getByTestId("output-status").textContent).toBe("Failed");
    expect(screen.getByText("exit 2")).toBeTruthy();
  });
});
