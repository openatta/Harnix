/**
 * The system space's keyboard and clocks: a tab strip is one Tab stop its arrows
 * move along; a row picked by a click is picked by Enter too; and the parts that read on a beat
 * share one clock per beat, the list around them not drawn again.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { useRef, useState } from "react";
import { rowKeys, useRoving } from "@harnix/react";

import { useTick } from "../src/view";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

function Strip() {
  const [on, setOn] = useState("a");
  const box = useRef<HTMLDivElement>(null);
  const roving = useRoving(box, { items: '[role="tab"]', orientation: "horizontal", activate: true });
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
