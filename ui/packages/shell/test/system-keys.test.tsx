/**
 * The system space's keyboard and clocks: a tab strip is one Tab stop its arrows
 * move along; a row picked by a click is picked by Enter too; and the parts that read on a beat
 * share one clock per beat, the list around them not drawn again.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { useRef, useState } from "react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider } from "@harnix/react";

import { rowKeys, useRovingTabIndex } from "../src/roving";
import { Shell } from "../src/Shell";
import { useTick } from "../src/system/view";
import { scriptedApp, tick } from "./scripted";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

function Strip() {
  const [on, setOn] = useState("a");
  const box = useRef<HTMLDivElement>(null);
  const roving = useRovingTabIndex(box, { items: '[role="tab"]', orientation: "horizontal", activate: true });
  return (
    <div role="tablist" ref={box} onKeyDown={roving.onKeyDown} onFocus={roving.onFocus}>
      {["a", "b", "c"].map((id) => (
        <button key={id} type="button" role="tab" data-tab={id} aria-selected={on === id} onClick={() => setOn(id)}>
          {id}
        </button>
      ))}
    </div>
  );
}

describe("roving tab index", () => {
  it("leaves one tab in the Tab order, the selected one, and the arrows move along and show", () => {
    const { container } = render(<Strip />);
    const tab = (id: string) => container.querySelector<HTMLElement>(`[data-tab="${id}"]`)!;
    expect([tab("a").tabIndex, tab("b").tabIndex, tab("c").tabIndex]).toEqual([0, -1, -1]);
    tab("a").focus();
    fireEvent.keyDown(tab("a"), { key: "ArrowRight" });
    expect(document.activeElement).toBe(tab("b"));
    expect(tab("b").getAttribute("aria-selected")).toBe("true");
    expect([tab("a").tabIndex, tab("b").tabIndex]).toEqual([-1, 0]);
    // Round from the first to the last, and End and Home.
    fireEvent.keyDown(tab("b"), { key: "End" });
    expect(document.activeElement).toBe(tab("c"));
    fireEvent.keyDown(tab("c"), { key: "ArrowRight" });
    expect(document.activeElement).toBe(tab("a"));
  });
});

describe("rows", () => {
  it("Enter and Space pick a row; Right and Left open and close it; Down goes to the next", () => {
    const pick = vi.fn();
    const set = vi.fn();
    const { container } = render(
      <div data-rows="">
        <div data-row="" tabIndex={0} data-testid="one" onKeyDown={rowKeys(pick, { open: false, set })} />
        <div data-row="" tabIndex={0} data-testid="two" onKeyDown={rowKeys(pick)} />
      </div>,
    );
    const one = container.querySelector<HTMLElement>('[data-testid="one"]')!;
    fireEvent.keyDown(one, { key: "Enter" });
    fireEvent.keyDown(one, { key: " " });
    expect(pick).toHaveBeenCalledTimes(2);
    fireEvent.keyDown(one, { key: "ArrowRight" });
    expect(set).toHaveBeenCalledWith(true);
    one.focus();
    fireEvent.keyDown(one, { key: "ArrowDown" });
    expect(document.activeElement).toBe(container.querySelector('[data-testid="two"]'));
  });
});

describe("useTick", () => {
  it("is one timer per beat, however many read it, and none once nobody does", () => {
    vi.useFakeTimers();
    const timers = vi.spyOn(globalThis, "setInterval");
    const drawn = { a: 0, b: 0, list: 0 };
    function Leaf({ name }: { name: "a" | "b" }) {
      useTick(3000);
      drawn[name] += 1;
      return null;
    }
    function List() {
      drawn.list += 1;
      return (
        <>
          <Leaf name="a" />
          <Leaf name="b" />
        </>
      );
    }
    const view = render(<List />);
    expect(timers).toHaveBeenCalledTimes(1);
    act(() => {
      vi.advanceTimersByTime(3000);
    });
    // Both leaves drew again on the beat; the list around them did not.
    expect(drawn).toEqual({ a: 2, b: 2, list: 1 });
    view.unmount();
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe("a space's sessions in the left column", () => {
  it("are read once, not again each time another of them is opened; again when the shell's own list is", async () => {
    const { app, client } = scriptedApp();
    // The scripted client's `notify` is its listeners: a session left would be detached through it.
    const listeners = (client as unknown as { notify: Set<unknown> }).notify;
    Object.assign(client, { notify: Object.assign(() => undefined, { add: (f: unknown) => listeners.add(f), delete: (f: unknown) => listeners.delete(f) }) });
    client.answer = (method) =>
      method === "harnix.sessions"
        ? { sessions: [{ session_id: "s1", instance: "system", name: "one" }, { session_id: "s2", instance: "system", name: "two" }] }
        : {};
    render(
      <HarnixUiProvider client={app.client as HarnixClient}>
        <Shell app={app} />
      </HarnixUiProvider>,
    );
    await act(async () => {
      await app.switchSpace("b");
      await tick();
    });
    const reads = () => client.calls.filter((c) => c.method === "harnix.sessions" && c.params.space === "b").length;
    const first = reads();
    expect(first).toBeGreaterThan(0);
    await act(async () => {
      app.open({ instance: "system", sessionId: "s1" });
      await tick();
      app.open({ instance: "system", sessionId: "s2" });
      await tick();
    });
    expect(reads()).toBe(first);
    await act(async () => {
      await app.refreshSessions();
      await tick();
    });
    expect(reads()).toBe(first + 1);
  });
});
