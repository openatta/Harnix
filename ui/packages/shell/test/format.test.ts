/**
 * The shell's writers of times, sizes and counts (`src/format.ts`), and what the system space says
 * of a task: each one figure written one way, wherever the shell shows it.
 */

import { afterEach, describe, expect, it, vi } from "vitest";

import { clock, compact, fileSize, megabytes, millis, since } from "../src/format";
import { taskDuration, usageText } from "../src/system/format";

afterEach(() => vi.useRealTimers());

describe("format", () => {
  it("since: seconds, then minutes, then hours and minutes", () => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000_000);
    expect(since(10_000_000 - 42_000)).toBe("42s");
    expect(since(10_000_000 - 5 * 60_000)).toBe("5m");
    expect(since(10_000_000 - (2 * 3600 + 5 * 60) * 1000)).toBe("2h05m");
    // A clock a little ahead of this one is not a negative time.
    expect(since(10_000_000 + 3000)).toBe("0s");
  });

  it("clock, millis and compact", () => {
    expect(clock(65)).toBe("1:05");
    expect(clock(3 * 3600 + 4 * 60 + 5)).toBe("3:04:05");
    expect(millis(640)).toBe("640ms");
    expect(millis(1500)).toBe("1.5s");
    expect(millis(125_000)).toBe("2m05s");
    expect(compact(980)).toBe("980");
    expect(compact(1234)).toBe("1.2k");
  });

  it("sizes: a file's in its own unit; memory always in MB", () => {
    expect(fileSize(undefined)).toBe("0 B");
    expect(fileSize(2048)).toBe("2.0 KB");
    expect(fileSize(3 * 1048576)).toBe("3.0 MB");
    expect(megabytes(undefined)).toBe("—");
    expect(megabytes(524288)).toBe("0.50 MB");
    expect(megabytes(32 * 1048576)).toBe("32 MB");
  });

  it("a task's duration and a call's usage", () => {
    expect(taskDuration({ task_id: "t", kind: "bash", status: "done", started_ms: 1000, ended_ms: 66_000 })).toBe("1m05s");
    expect(taskDuration({ task_id: "t", kind: "bash", status: "pending" })).toBe("");
    expect(usageText(null)).toBe("—");
    expect(usageText({ input_tokens: 34, cache_read_input_tokens: 1000, output_tokens: 200 })).toBe("in 1.0k · out 200");
  });
});
