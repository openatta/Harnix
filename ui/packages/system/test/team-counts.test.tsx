/** A team's counts are said as whole phrases in each language, not a state and a number side by side. */

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { setLocale } from "@harnix/react";

import { TeamBoard } from "../src/pane/tasks";
import { taskLabel } from "../src/format";

afterEach(() => {
  cleanup();
  setLocale("en");
});

const team = { team: "review", team_id: "t1", stage: "read", stage_index: 0, stage_count: 2, status: "running", members: ["a", "b", "c"], failed: ["c"] };

describe("a team's counts", () => {
  it("read as phrases: '2 running · 1 failed', '2 个运行中 · 1 个失败'", () => {
    setLocale("en");
    render(<TeamBoard teams={[team]} tasks={[]} />);
    expect(screen.getByTestId("team-board").querySelector(".team-h .note")!.textContent).toBe("2 running · 1 failed");
    cleanup();
    setLocale("zh-CN");
    render(<TeamBoard teams={[team]} tasks={[]} />);
    expect(screen.getByTestId("team-board").querySelector(".team-h .note")!.textContent).toBe("2 个运行中 · 1 个失败");
  });

  it("a standing member between two messages is idle, and says so", () => {
    setLocale("en");
    const task = { task_id: "member-1", kind: "member", status: "idle", started_ms: 1, ended_ms: null, detail: { team: "review", member: "a" } };
    render(<TeamBoard teams={[team]} tasks={[task]} />);
    const row = screen.getByTestId("team-board").querySelector('.member[data-member="a"]')!;
    expect(row.textContent).toContain("Idle");
    expect(screen.getByTestId("team-board").querySelector(".team-h .note")!.textContent).toContain("Idle 1");
  });
});

describe("a task's label", () => {
  it("an MCP task is the server and the tool", () => {
    const task = { task_id: "mcp-1", kind: "mcp_task", status: "running", started_ms: 1, ended_ms: null, detail: { server: "docs", tool: "search" } };
    expect(taskLabel(task as never)).toBe("docs search");
  });
});
