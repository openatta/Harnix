/**
 * Going back from a message's menu (`session.action` at `message`) or a file tool's row: the menu
 * offers 「回到这里」, greyed with the reason while a turn runs; the dialog shows the dry run — turns,
 * files restored and deleted, conflicts, files it cannot restore — and lets a rewind through only
 * once what is in the way is overwritten or skipped as the person chose; the rewind is asked with
 * their choices, and the message goes back into the composer.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import type { HarnixClient, RewindResult } from "@harnix/core";
import { HarnixUiProvider, type ActionTarget, type ComposerApi } from "@harnix/react";

import { MessageMenu } from "../src/actions";
import { registerBuiltins } from "../src/builtins";
import { closeMenu, openedMenu } from "../src/Menu";
import { rewindDialog } from "../src/rewind";
import { scriptedApp, tick } from "./scripted";

afterEach(() => {
  act(() => closeMenu());
  cleanup();
});

const session = { instance: "main", sessionId: "s1" };
const message: ActionTarget = { kind: "message", session, space: "sessions", turnId: "t2", text: "fix a" };

function plan(extra: Partial<RewindResult> = {}): RewindResult {
  return {
    instance: "main",
    session_id: "s1",
    rewind_id: "r1",
    dry_run: true,
    scope: "both",
    to: { turn: "t1" },
    conversation: { messages_before: 6, messages_after: 2, removed_turns: [{ turn_id: "t2", prompt: "fix a" }, { turn_id: "t3", prompt: "and b" }] },
    files: {
      restored: [{ path: "src/a.rs", to: "file" }, { path: "src/new.rs", to: "absent" }],
      skipped: [{ path: "src/b.rs", issue: "modified", detail: "changed by hand", overridable_by: "force" }],
      overwrote_external: [],
    },
    ...extra,
  };
}

function setUp(answer: (params: Record<string, unknown>) => unknown) {
  const { app, client } = scriptedApp();
  registerBuiltins(app);
  app.open(session);
  client.answer = (method, params) => (method === "harnix.session.rewind" ? answer(params) : {});
  return { app, client };
}

/** The dialog the shell opened, drawn. */
function drawnDialog(app: ReturnType<typeof scriptedApp>["app"]) {
  const content = app.snapshot.modal?.content;
  return render(<HarnixUiProvider client={app.conn as HarnixClient}>{content}</HarnixUiProvider>);
}

describe("a message's menu", () => {
  it("offers going back to it, and greys it with the reason while a turn runs", () => {
    const { app } = setUp(() => plan());
    const page = render(
      <HarnixUiProvider client={app.conn as HarnixClient}>
        <MessageMenu app={app} target={message} />
      </HarnixUiProvider>,
    );
    fireEvent.click(page.getByTestId("conv-message-menu"));
    const item = openedMenu()!.items.find((i) => i.id === "action:builtin.rewind")!;
    expect(item.disabled).toBe(false);
    act(() => closeMenu());

    app.controller = { getSnapshot: () => ({ turn: { turnId: "t4" }, queue: [] }) } as never;
    fireEvent.click(page.getByTestId("conv-message-menu"));
    const busy = openedMenu()!.items.find((i) => i.id === "action:builtin.rewind")!;
    expect(busy.disabled).toBe(true);
    expect(busy.sub).toBeTruthy();
  });

  it("has no button when nothing is offered for the message", () => {
    const { app } = scriptedApp();
    const page = render(
      <HarnixUiProvider client={app.conn as HarnixClient}>
        <MessageMenu app={app} target={message} />
      </HarnixUiProvider>,
    );
    expect(page.queryByTestId("conv-message-menu")).toBeNull();
  });

  it("a file tool's row offers its files back; another tool's does not", () => {
    const { app } = setUp(() => plan());
    const write: ActionTarget = { kind: "tool", session, space: "sessions", toolUseId: "u1", tool: "Write" };
    const read: ActionTarget = { ...write, tool: "Read" };
    expect(app.registry.actions("tool", write).map((a) => a.id)).toContain("builtin.rewind-files");
    expect(app.registry.actions("tool", read).map((a) => a.id)).not.toContain("builtin.rewind-files");
  });
});

describe("the rewind dialog", () => {
  it("shows the dry run, holds back until a conflict is overwritten, rewinds as chosen and gives the message back", async () => {
    const asked: Record<string, unknown>[] = [];
    const { app } = setUp((params) => {
      asked.push(params);
      return params.dry_run ? plan() : plan({ dry_run: false, fork_session_id: "s9" });
    });
    const inserted: string[] = [];
    app.composerBound("main/s1", { insert: (text: string) => inserted.push(text) } as unknown as ComposerApi);
    let done = false;
    const closed = rewindDialog(app, message, { before_turn: "t2" }).then(() => {
      done = true;
    });
    const page = drawnDialog(app);
    await act(tick);

    expect(asked[0]).toEqual({ instance: "main", session_id: "s1", space: "sessions", to: { before_turn: "t2" }, dry_run: true });
    expect(page.getByTestId("rewind-dialog").dataset.state).toBe("planned");
    expect([...page.getByTestId("rewind-turns").querySelectorAll("li")].map((li) => li.textContent)).toEqual(["fix a", "and b"]);
    const restored = [...page.getByTestId("rewind-restore").querySelectorAll("li")].map((li) => [li.dataset.path, li.dataset.to]);
    expect(restored).toEqual([["src/a.rs", "file"], ["src/new.rs", "absent"]]);
    expect(page.getByTestId("rewind-conflicts").querySelector("li")!.dataset.path).toBe("src/b.rs");
    expect(page.queryByTestId("rewind-issues")).toBeNull();

    const go = page.getByTestId("rewind-confirm") as HTMLButtonElement;
    expect(go.disabled).toBe(true);
    fireEvent.click(page.getByTestId("rewind-force"));
    fireEvent.click(page.getByTestId("rewind-fork"));
    expect(go.disabled).toBe(false);
    fireEvent.click(go);
    await act(tick);
    await closed;

    expect(asked[1]).toEqual({ instance: "main", session_id: "s1", space: "sessions", to: { before_turn: "t2" }, fork_first: true, force: true });
    expect(done).toBe(true);
    expect(inserted).toEqual(["fix a"]);
    expect(app.snapshot.modal).toBeNull();
  });

  it("files it cannot restore are skipped only when the person says so; a refusal is shown, not swallowed", async () => {
    const unavailable = plan({ files: { restored: [], skipped: [{ path: "big.bin", issue: "unavailable", detail: "too large", overridable_by: "allow_partial" }], overwrote_external: [] } });
    const { app } = setUp((params) => {
      if (params.dry_run) return unavailable;
      throw { code: -32015, message: "busy" };
    });
    void rewindDialog(app, message, { before_turn: "t2" });
    const page = drawnDialog(app);
    await act(tick);
    expect(page.getByTestId("rewind-issues").querySelector("li")!.dataset.issue).toBe("unavailable");
    const go = page.getByTestId("rewind-confirm") as HTMLButtonElement;
    expect(go.disabled).toBe(true);
    fireEvent.click(page.getByTestId("rewind-skip"));
    fireEvent.click(go);
    await act(tick);
    expect(page.getByTestId("rewind-error").textContent).toContain("busy");
    expect(app.snapshot.modal).not.toBeNull();
  });

  it("a dry run that fails says why and offers nothing to confirm", async () => {
    const { app } = setUp(() => {
      throw { code: -32602, message: "this session keeps no checkpoints" };
    });
    void rewindDialog(app, message, { before_turn: "t2" });
    const page = drawnDialog(app);
    await act(tick);
    expect(page.getByTestId("rewind-dialog").dataset.state).toBe("failed");
    expect(page.getByTestId("rewind-error").textContent).toContain("checkpoints");
    expect((page.getByTestId("rewind-confirm") as HTMLButtonElement).disabled).toBe(true);
  });
});
