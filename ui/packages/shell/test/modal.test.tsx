/** `host.modal` (ipdrun-38): the shell's modal layer, and Escape while it is open. */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider } from "@harnix/react";

import { Shell } from "../src/Shell";
import { hostFor } from "../src/host";
import { scriptedApp } from "./scripted";

afterEach(cleanup);

function escape(target: EventTarget) {
  act(() => {
    target.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  });
}

describe("modal", () => {
  it("says when the shell's modal opens and closes", () => {
    const { app } = scriptedApp();
    const host = hostFor(app);
    const seen: boolean[] = [];
    const off = host.modal.onChange((open) => seen.push(open));
    expect(host.modal.isOpen()).toBe(false);
    app.openModal({ title: "one", content: null });
    app.openModal({ title: "two", content: null }); // still open: not news
    expect(host.modal.isOpen()).toBe(true);
    app.closeModal();
    off();
    app.openModal({ title: "three", content: null });
    expect(seen).toEqual([true, false]);
  });

  it("Escape closes only the modal: a space's document listener does not see it; without a modal it does", () => {
    const { app } = scriptedApp();
    render(
      <HarnixUiProvider client={app.client as HarnixClient}>
        <Shell app={app} />
      </HarnixUiProvider>,
    );
    const heard: string[] = [];
    const listener = (e: KeyboardEvent) => heard.push(e.key);
    document.addEventListener("keydown", listener);
    try {
      // From the page, focus outside the modal.
      act(() => app.openModal({ title: "m", content: <input data-testid="in-modal" /> }));
      escape(document.body);
      expect(app.snapshot.modal).toBeNull();
      expect(heard).toEqual([]);

      // From inside the modal.
      act(() => app.openModal({ title: "m", content: <input data-testid="in-modal" /> }));
      const input = document.querySelector('[data-testid="in-modal"]')!;
      escape(input);
      expect(app.snapshot.modal).toBeNull();
      expect(heard).toEqual([]);

      // No modal: unchanged, the space hears it.
      escape(document.body);
      expect(heard).toEqual(["Escape"]);
    } finally {
      document.removeEventListener("keydown", listener);
    }
  });
});
