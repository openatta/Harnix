/**
 * The system space's registration: the two points it defines, the tabs other packages put on them,
 * the actions it offers on a session and a tool call, and the session menu's actions in a session's
 * operations — the way an export reaches the extension that has one.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import { Registry, RegistryProvider, setLocale, tabsOf, type ActionTarget, type SpaceContribution } from "@harnix/react";

import { register } from "../src/index";
import { fakeHost } from "./fake-host";

afterEach(() => {
  cleanup();
  setLocale("en");
});

const tick = () => new Promise((r) => setTimeout(r, 0));

function registered() {
  const fake = fakeHost();
  const r = new Registry();
  r.activate("system", () => register(r, fake.host));
  return { fake, r, space: () => r.at("space").find((c) => c.id === "system") as SpaceContribution & { owner: string } };
}

describe("the system space's points", () => {
  it("defines the console's tabs and the inspector's", () => {
    const { r } = registered();
    expect(r.isDefined("system.console.tab")).toBe(true);
    expect(r.isDefined("system.inspector.tab")).toBe(true);
    expect(r.at("session.action").map((a) => a.id)).toEqual(["system.open", "system.output", "system.tool"]);
  });

  it("draws another package's console tab among the Agents tabs, by its order", async () => {
    const { r, space } = registered();
    r.register("system.console.tab", { id: "later", order: 20, title: () => "Later", render: () => <div data-testid="later-page">later</div> }, "other");
    r.register("system.console.tab", { id: "first", order: 10, title: () => "First", render: () => "first" }, "other");
    r.settle();
    render(<RegistryProvider registry={r}>{space().center!.overview!()}</RegistryProvider>);
    const tabs = [...document.querySelectorAll('[role="tab"]')].map((el) => el.getAttribute("data-tab"));
    // Its own extension log is one of them, after the others' by its order.
    expect(tabs).toEqual(["active", "all", "first", "later", "system.extlog"]);
    await act(async () => {
      (document.querySelector('[data-tab="later"]') as HTMLElement).click();
      await tick();
    });
    expect(screen.getByTestId("later-page").textContent).toBe("later");
  });

  it("puts another owner's inspector tab on a session, after the overview, and opens the one for tool calls from a tool row", async () => {
    const { fake, r, space } = registered();
    r.register(
      "system.inspector.tab",
      { id: "probe.calls", tools: true, title: () => "Calls", match: (t: { kind: string }) => t.kind === "session", render: (t: { tool?: string }) => <div data-testid="probe">{t.tool ?? "none"}</div> },
      "probe",
    );
    r.register("system.inspector.tab", { id: "probe.never", title: () => "Never", match: () => false, render: () => null }, "probe");
    r.settle();
    const target: ActionTarget = { kind: "tool", session: { instance: "main", sessionId: "s1" }, space: "sessions", toolUseId: "tu-1", tool: "Bash" };
    const tool = r.at("session.action").find((a) => a.id === "system.tool")!;
    expect(tool.match(target)).toBe(true);
    await act(async () => {
      await tool.run(target);
    });
    expect(fake.shown.space).toBe("system");
    expect(fake.shown.tab).toBe("probe.calls");
    const tabs = tabsOf(space());
    expect(tabs.map((x) => x.id)).toEqual(["system.overview", "probe.calls", "system.context", "system.tasks", "system.frames"]);
    render(<RegistryProvider registry={r}>{tabs[1]!.render()}</RegistryProvider>);
    expect(screen.getByTestId("probe").textContent).toBe("tu-1");
  });

  it("offers the session menu's actions in a session's operations, asked about the session in its own space", async () => {
    const { fake, r, space } = registered();
    const ran: ActionTarget[] = [];
    r.register(
      "session.action",
      { id: "probe.export", at: ["menu"], label: () => "Export", match: (t: ActionTarget) => t.space === "sessions", run: (t: ActionTarget) => void ran.push(t) },
      "probe",
    );
    r.settle();
    fake.answer = (method) => (method === "harnix.sessions.list" ? { sessions: [{ session_id: "s1", instance: "main", space: "sessions", name: "one" }] } : null);
    // From the conversation: that space's sessions, theirs first; picking it shows its operations.
    await act(async () => {
      await r.at("session.action").find((a) => a.id === "system.open")!.run({ kind: "session", session: { instance: "main", sessionId: "s1" }, space: "sessions" });
    });
    render(<RegistryProvider registry={r}>{tabsOf(space())[0]!.render()}</RegistryProvider>);
    await act(async () => {
      await tick();
    });
    await act(async () => {
      screen.getByTestId("space-session").click();
      await tick();
    });
    cleanup();
    render(<RegistryProvider registry={r}>{tabsOf(space())[0]!.render()}</RegistryProvider>);
    await act(async () => {
      fake.push("harnix.instances.sessions", { sessions: [{ session_id: "s1", space: "sessions", name: "one" }] });
      await tick();
    });
    // No export of its own: the one offered on the session menu, run against the session's own space.
    expect(screen.queryByTestId("op-export")).toBeNull();
    await act(async () => {
      screen.getByTestId("op-action-probe.export").click();
      await tick();
    });
    expect(ran).toEqual([{ kind: "session", session: { instance: "main", sessionId: "s1" }, space: "sessions" }]);
  });

  it("reads a session's facts again once an operation is done: the model it was switched to is named", async () => {
    const { fake, r, space } = registered();
    let model = "model-a";
    fake.answer = (method, params) => {
      if (method === "harnix.sessions.list") return { sessions: [{ session_id: "s1", instance: "main", space: "sessions", name: "one" }] };
      if (method === "system.model") model = (params as { model: string }).model;
      if (method === "system.context") return { model };
      return null;
    };
    fake.host.dialogs.prompt = async () => "model-b";
    await act(async () => {
      await r.at("session.action").find((a) => a.id === "system.open")!.run({ kind: "session", session: { instance: "main", sessionId: "s1" }, space: "sessions" });
    });
    render(<RegistryProvider registry={r}>{tabsOf(space())[0]!.render()}</RegistryProvider>);
    await act(async () => {
      await tick();
    });
    await act(async () => {
      screen.getByTestId("space-session").click();
      await tick();
    });
    cleanup();
    render(<RegistryProvider registry={r}>{tabsOf(space())[0]!.render()}</RegistryProvider>);
    await act(async () => {
      fake.push("harnix.instances.sessions", { sessions: [{ session_id: "s1", space: "sessions", name: "one" }] });
      await tick();
    });
    expect(screen.getByTestId("system-session").textContent).toContain("model-a");
    await act(async () => {
      screen.getByTestId("op-model").click();
      await tick();
      await tick();
    });
    expect(screen.getByTestId("system-session").textContent).toContain("model-b");
  });
});
