/** A `/` command that takes what follows it (`CommandCandidate.submit`): handed the arguments and where the composer is; a failure is the shell's error and keeps the text. */

import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { useCommands } from "../src/commands";
import { scriptedApp } from "./scripted";

describe("a / command with submit", () => {
  it("gets the arguments and the context; a throw is a notice and keeps the text", async () => {
    const { app } = scriptedApp();
    const submit = vi.fn(async (args: string) => {
      if (args === "bad") throw new Error("no such flow");
      return true;
    });
    app.registry.register("command", { id: "takes", complete: () => [{ replacement: "/take", label: "take", submit }, { replacement: "/plain", label: "plain" }] }, "test");
    const notice = vi.spyOn(app, "notice");
    const { result } = renderHook(() => useCommands(app));
    await waitFor(() => expect(result.current.map((c) => c.name)).toEqual(["take", "plain"]));
    expect(result.current[1]!.submit).toBeUndefined();
    let answer: unknown;
    await act(async () => {
      answer = await result.current[0]!.submit!("some words");
    });
    expect(answer).toBe(true);
    expect(submit).toHaveBeenCalledWith("some words", { space: app.snapshot.space, session: null });
    await act(async () => {
      answer = await result.current[0]!.submit!("bad");
    });
    expect(answer).toBe(false);
    expect(notice).toHaveBeenCalledWith(expect.stringContaining("no such flow"), "error");
  });
});
