/**
 * A tool row's argument as a door (`session.action` at `arg`): the first action that matches the
 * call makes it a link, handed the call's input; a click runs it and leaves the row as it was. With
 * none, the argument is plain text, as before.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import type { BlockOf } from "@harnix/core/state";
import { HarnixUiProvider, type ActionTarget } from "@harnix/react";

import { registerBuiltins } from "../src/builtins";
import { scriptedApp, tick } from "./scripted";

afterEach(cleanup);

const read: BlockOf<"tool"> = { key: 1, kind: "tool", id: "u1", name: "Read", input: { file_path: "/w/src/a.rs", offset: 4 }, status: "ok" };
const bash: BlockOf<"tool"> = { key: 2, kind: "tool", id: "u2", name: "Bash", input: { command: "ls" }, status: "ok" };

function drawn(block: BlockOf<"tool">) {
  const { app } = scriptedApp();
  registerBuiltins(app);
  app.open({ instance: "main", sessionId: "s1" });
  const ran: ActionTarget[] = [];
  app.registry.register(
    "session.action",
    { id: "test.open", at: ["arg"], label: () => "Open it", match: (t) => t.kind === "tool" && t.tool === "Read", run: (t) => void ran.push(t) },
    "test",
  );
  app.registry.register("session.action", { id: "test.later", at: ["arg"], label: () => "Later", match: () => true, run: () => undefined }, "test");
  const row = app.registry.claim("tool.row", block)!;
  const page = render(<HarnixUiProvider client={app.conn as HarnixClient}>{row.render(block)}</HarnixUiProvider>);
  return { page, ran };
}

describe("a tool row's argument", () => {
  it("is a link to the first action at `arg` that matches, handed the call's input", async () => {
    const { page, ran } = drawn(read);
    const arg = page.getByTestId("conv-tool-args");
    expect([arg.dataset.action, arg.getAttribute("title"), arg.textContent]).toEqual(["test.open", "Open it", "/w/src/a.rs"]);
    fireEvent.click(arg);
    await act(tick);
    expect(ran).toEqual([{ kind: "tool", session: { instance: "main", sessionId: "s1" }, space: expect.any(String), toolUseId: "u1", tool: "Read", input: read.input }]);
    // The row did not open for it.
    expect(page.getByTestId("conv-tool-toggle").getAttribute("aria-expanded")).toBe("false");
  });

  it("is the action that matches, and plain text when none does", () => {
    const first = drawn(bash).page;
    expect(first.getByTestId("conv-tool-args").dataset.action).toBe("test.later");
    first.unmount();
    const { app } = scriptedApp();
    registerBuiltins(app);
    app.open({ instance: "main", sessionId: "s1" });
    const page = render(<HarnixUiProvider client={app.conn as HarnixClient}>{app.registry.claim("tool.row", bash)!.render(bash)}</HarnixUiProvider>);
    expect(page.getByTestId("conv-tool-args").dataset.action).toBeUndefined();
  });
});
