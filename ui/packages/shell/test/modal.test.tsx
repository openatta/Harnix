/**
 * The page's layers (`layers.ts`): the shell's dialogs are a stack — the focus in the top one, the
 * page under it inert, Escape closing that one alone, the shortcuts quiet — and `host.modal`
 * (ipdrun-38) says when the stack opens and closes. A menu is a layer too.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider } from "@harnix/react";

import { Shell } from "../src/Shell";
import { hostFor } from "../src/host";
import { closeMenu, openMenu, openedMenu } from "../src/Menu";
import { scriptedApp } from "./scripted";

afterEach(() => {
  act(() => closeMenu());
  cleanup();
});

function key(target: EventTarget, k: string, extra: KeyboardEventInit = {}) {
  act(() => {
    target.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...extra }));
  });
}

function mount() {
  const { app } = scriptedApp();
  render(
    <HarnixUiProvider client={app.client as HarnixClient}>
      <Shell app={app} />
    </HarnixUiProvider>,
  );
  return app;
}

describe("modal", () => {
  it("says when the shell's modal layer opens and closes, not when another dialog goes over it", () => {
    const { app } = scriptedApp();
    const host = hostFor(app);
    const seen: boolean[] = [];
    const off = host.modal.onChange((open) => seen.push(open));
    expect(host.modal.isOpen()).toBe(false);
    app.openModal({ title: "one", content: null });
    app.openModal({ title: "two", content: null }); // over the first: still open, not news
    expect(host.modal.isOpen()).toBe(true);
    expect(app.snapshot.modal?.title).toBe("two");
    app.closeModal(); // the top one
    expect(app.snapshot.modal?.title).toBe("one");
    app.closeModal();
    off();
    app.openModal({ title: "three", content: null });
    expect(seen).toEqual([true, false]);
  });

  it("a dialog with the key of the one on top takes its place", () => {
    const { app } = scriptedApp();
    app.openModal({ title: "settings", key: "settings", content: null });
    app.openModal({ title: "settings again", key: "settings", content: null });
    expect(app.snapshot.modals.map((m) => m.title)).toEqual(["settings again"]);
  });

  it("Escape closes only the modal: a space's document listener does not see it; without a modal it does", () => {
    const app = mount();
    const heard: string[] = [];
    const listener = (e: KeyboardEvent) => heard.push(e.key);
    document.addEventListener("keydown", listener);
    try {
      // From the page, focus outside the modal.
      act(() => app.openModal({ title: "m", content: <input data-testid="in-modal" /> }));
      key(document.body, "Escape");
      expect(app.snapshot.modal).toBeNull();
      expect(heard).toEqual([]);

      // From inside the modal.
      act(() => app.openModal({ title: "m", content: <input data-testid="in-modal" /> }));
      key(document.querySelector('[data-testid="in-modal"]')!, "Escape");
      expect(app.snapshot.modal).toBeNull();
      expect(heard).toEqual([]);

      // No modal: unchanged, the space hears it.
      key(document.body, "Escape");
      expect(heard).toEqual(["Escape"]);
    } finally {
      document.removeEventListener("keydown", listener);
    }
  });

  it("Escape closes the top dialog alone, from inside it or from the page", () => {
    const app = mount();
    act(() => app.openModal({ title: "under", content: <input data-testid="under" /> }));
    act(() => app.openModal({ title: "over", content: <input data-testid="over" /> }));
    key(document.querySelector('[data-testid="over"]')!, "Escape");
    expect(app.snapshot.modals.map((m) => m.title)).toEqual(["under"]);
    act(() => app.openModal({ title: "over", content: <input data-testid="over" /> }));
    key(window, "Escape");
    expect(app.snapshot.modals.map((m) => m.title)).toEqual(["under"]);
    key(window, "Escape");
    expect(app.snapshot.modals).toEqual([]);
  });

  it("the focus goes into a dialog, round inside it, and back when it closes; the page under it is inert", () => {
    const app = mount();
    const opener = document.createElement("button");
    document.getElementById("side")!.append(opener);
    opener.focus();
    act(() =>
      app.openModal({
        title: "m",
        content: (
          <div className="body">
            <input data-testid="a" />
            <button type="button" data-testid="b">
              b
            </button>
          </div>
        ),
      }),
    );
    const a = document.querySelector<HTMLElement>('[data-testid="a"]')!;
    const b = document.querySelector<HTMLElement>('[data-testid="b"]')!;
    expect(document.activeElement).toBe(a);
    expect(document.getElementById("frame")!.inert).toBe(true);
    // From the last stop forward, round to the first (the dialog's × comes first in it).
    b.focus();
    key(b, "Tab");
    expect(document.activeElement?.closest("#modal")).not.toBeNull();
    expect(document.activeElement).not.toBe(b);
    // Shift+Tab from the first stop goes round to the last.
    const first = document.activeElement as HTMLElement;
    key(first, "Tab", { shiftKey: true });
    expect(document.activeElement).toBe(b);

    // Another dialog over it: the one under goes inert too.
    act(() => app.openModal({ title: "over", content: <input data-testid="c" /> }));
    expect(document.getElementById("modal")!.inert).toBe(true);
    expect(document.activeElement).toBe(document.querySelector('[data-testid="c"]'));
    act(() => app.closeModal());
    expect(document.getElementById("modal")!.inert).toBe(false);
    expect(document.activeElement).toBe(b);

    act(() => app.closeModal());
    expect(document.getElementById("frame")!.inert).toBe(false);
    expect(document.activeElement).toBe(opener);
  });

  it("the shortcuts do nothing while a dialog is open", () => {
    const app = mount();
    const before = app.snapshot.sidebar;
    act(() => app.openModal({ title: "m", content: null }));
    key(window, "\\", { metaKey: true });
    expect(app.snapshot.sidebar).toBe(before);
    act(() => app.closeModal());
    key(window, "\\", { metaKey: true });
    expect(app.snapshot.sidebar).not.toBe(before);
  });

  it("a menu is a layer too: Escape closes it before the dialog under it, and a dialog opening closes it", () => {
    const app = mount();
    act(() => openMenu({ label: "m", items: [{ id: "x", label: "x", run: () => undefined }], at: { x: 10, y: 10 }, testId: "test-menu" }));
    const menu = document.querySelector<HTMLElement>('[data-testid="test-menu"]')!;
    expect(menu.getAttribute("role")).toBe("menu");
    expect(document.activeElement).toBe(menu.querySelector('[data-item="x"]'));
    act(() => app.openModal({ title: "m", content: null }));
    expect(openedMenu()).toBeNull();
    act(() => app.closeModal());

    act(() => openMenu({ label: "m", items: [{ id: "x", label: "x", run: () => undefined }], at: { x: 10, y: 10 }, testId: "test-menu" }));
    key(document.activeElement!, "Escape");
    expect(openedMenu()).toBeNull();
  });

  it("a menu's items are walked with the arrow keys, and Enter runs one and closes it", () => {
    mount();
    const ran: string[] = [];
    const items = ["a", "b", "c"].map((id) => ({ id, label: id, run: () => ran.push(id) }));
    act(() => openMenu({ label: "m", items, at: { x: 10, y: 10 }, testId: "test-menu" }));
    const item = (id: string) => document.querySelector<HTMLElement>(`[data-testid="test-menu"] [data-item="${id}"]`);
    expect(document.activeElement).toBe(item("a"));
    key(document.activeElement!, "ArrowDown");
    expect(document.activeElement).toBe(item("b"));
    key(document.activeElement!, "ArrowUp");
    key(document.activeElement!, "ArrowUp");
    expect(document.activeElement).toBe(item("c"));
    act(() => (document.activeElement as HTMLElement).click());
    expect(ran).toEqual(["c"]);
    expect(openedMenu()).toBeNull();
  });
});
