/** What the system space says of a task and a call: each figure written one way. */

import { describe, expect, it } from "vitest";

import { taskDuration, usageText } from "../src/format";

describe("format", () => {
  it("a task's duration and a call's usage", () => {
    expect(taskDuration({ task_id: "t", kind: "bash", status: "done", started_ms: 1000, ended_ms: 66_000 })).toBe("1m05s");
    expect(taskDuration({ task_id: "t", kind: "bash", status: "pending" })).toBe("");
    expect(usageText(null)).toBe("—");
    expect(usageText({ input_tokens: 34, cache_read_input_tokens: 1000, output_tokens: 200 })).toBe("in 1.0k · out 200");
  });
});
