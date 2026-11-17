/**
 * A person's message, changed from its menu: rewritten, or taken out of the context. Found by its
 * turn in the outline, changed against the count of messages the history had then, claiming the
 * backend space its interface space lists; not while a turn runs or waits.
 */

import type { ReactNode } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { Registry, type ActionTarget } from "@harnix/react";

import { register } from "../src/index";
import { scripted } from "./host";

afterEach(cleanup);

const tick = () => new Promise((r) => setTimeout(r, 0));
const session = { instance: "main", sessionId: "s1" };
const target: ActionTarget = { kind: "message", session, space: "projects", turnId: "t2", text: "fix a" };
const OUTLINE = { instance: "main", session_id: "s1", total: 6, turns: [{ turn_id: "t1", seq: 0 }, { turn_id: "t2", seq: 3 }] };

function registered(options: { running?: boolean; confirmed?: boolean } = {}) {
  const s = scripted((method) => (method === "harnix.session.outline" ? OUTLINE : method === "observability.messages.edit" ? { edited: 3 } : method === "observability.messages.drop" ? { dropped: [3] } : {}));
  const dialogs: ((close: (v: unknown) => void) => ReactNode)[] = [];
  const closed: unknown[] = [];
  const notices: string[] = [];
  Object.assign(s.host, {
    session: {
      current: () => session,
      context: () => ({ turn: options.running ? { id: "t3" } : null, session: { turn_state: "idle" }, queue: [] }),
    },
    dialogs: {
      confirm: async () => options.confirmed ?? true,
      open: (render: (close: (v: unknown) => void) => ReactNode) =>
        new Promise((resolve) => {
          dialogs.push((close) => render((v) => (closed.push(v), close(v), resolve(v))));
        }),
    },
    shell: { notice: (text: string) => void notices.push(text) },
  });
  const r = new Registry();
  // The interface space `projects` lists the backend space `sessions`.
  r.register("space", { id: "projects", icon: "folder", title: () => "Projects", sessions: "sessions" }, "test");
  r.activate("observability", () => register(r, s.host));
  r.settle();
  const action = (id: string) => r.at("session.action").find((a) => a.id === id)!;
  return { s, r, action, dialogs, closed, notices };
}

describe("a person's message's menu", () => {
  it("offers a rewrite and a taking-out on a person's message only, not while a turn runs", () => {
    const { action } = registered();
    for (const id of ["observability.message.edit", "observability.message.drop"]) {
      expect(action(id).at).toEqual(["message"]);
      expect(action(id).match(target)).toBe(true);
      expect(action(id).match({ kind: "tool", session, space: "projects", toolUseId: "u", tool: "Bash" })).toBe(false);
      expect(action(id).disabled!(target)).toBeFalsy();
    }
    const busy = registered({ running: true });
    expect(busy.action("observability.message.edit").disabled!(target)).toBe("Not while a turn runs or waits");
    expect(busy.action("observability.message.drop").disabled!(target)).toBe("Not while a turn runs or waits");
  });

  it("rewrites the message where its turn starts, against the count the history had", async () => {
    const { s, action, dialogs, notices } = registered();
    let ran!: Promise<void>;
    act(() => {
      ran = action("observability.message.edit").run(target) as Promise<void>;
    });
    await act(tick);
    render(<>{dialogs[0]!(() => undefined)}</>);
    const save = screen.getByTestId("message-edit-save") as HTMLButtonElement;
    // Unchanged words change nothing.
    expect(save.disabled).toBe(true);
    fireEvent.change(screen.getByTestId("message-edit-text"), { target: { value: "fix a, then b" } });
    await act(async () => {
      fireEvent.click(screen.getByTestId("message-edit-save"));
      await tick();
    });
    await act(async () => void (await ran));
    expect(s.calls.find((c) => c.method === "harnix.session.outline")?.params).toEqual({ instance: "main", session_id: "s1", space: "sessions" });
    expect(s.calls.find((c) => c.method === "observability.messages.edit")?.params).toEqual({ instance: "main", session_id: "s1", space: "sessions", at: 3, text: "fix a, then b", total: 6 });
    expect(notices).toEqual(["The message was rewritten."]);
  });

  it("says what went wrong in the dialog, and keeps it open", async () => {
    const { s, action, dialogs } = registered();
    const refused = Object.assign(new Error("total does not match: the history has 7 messages"), { code: -32602 });
    const answer = s.host.call;
    s.host.call = (async (method: string, params: unknown) => (method === "observability.messages.edit" ? Promise.reject(refused) : answer(method, params))) as typeof s.host.call;
    act(() => void action("observability.message.edit").run(target));
    await act(tick);
    render(<>{dialogs[0]!(() => undefined)}</>);
    fireEvent.change(screen.getByTestId("message-edit-text"), { target: { value: "other words" } });
    await act(async () => {
      fireEvent.click(screen.getByTestId("message-edit-save"));
      await tick();
    });
    expect(screen.getByTestId("message-edit-error").textContent).toMatch(/total does not match/);
    expect((screen.getByTestId("message-edit-save") as HTMLButtonElement).disabled).toBe(false);
  });

  it("takes the message out once confirmed, and not when not", async () => {
    const no = registered({ confirmed: false });
    await no.action("observability.message.drop").run(target);
    expect(no.s.calls.some((c) => c.method === "observability.messages.drop")).toBe(false);
    const { s, action, notices } = registered();
    await action("observability.message.drop").run(target);
    expect(s.calls.find((c) => c.method === "observability.messages.drop")?.params).toEqual({ instance: "main", session_id: "s1", space: "sessions", at: [3], total: 6 });
    expect(notices).toEqual(["The message was taken out of the context."]);
  });

  it("refuses a message whose turn is no longer in the conversation", async () => {
    const { s, action } = registered();
    await expect(action("observability.message.drop").run({ ...target, turnId: "t9" })).rejects.toThrow("This message is no longer in the conversation.");
    expect(s.calls.some((c) => c.method === "observability.messages.drop")).toBe(false);
  });
});
