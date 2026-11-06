/** A space's second side panel (xops-04 §7): beside the conversation, resizable, closable. */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider, type SpaceSecondary } from "@harnix/react";

import { secondaryWidthOf, WIDTHS } from "../src/app";
import { Shell } from "../src/Shell";
import { scriptedApp, space } from "./scripted";

afterEach(cleanup);

function drawn(panel: () => SpaceSecondary | null) {
  const { app } = scriptedApp();
  app.registry.register("space", space("side", { secondary: panel }), "test");
  app.set({ space: "side" });
  const { container } = render(
    <HarnixUiProvider client={app.client as HarnixClient}>
      <Shell app={app} />
    </HarnixUiProvider>,
  );
  const frame = () => container.querySelector<HTMLElement>("#frame")!;
  const aside = () => container.querySelector<HTMLElement>("#secondary");
  return { app, container, frame, aside };
}

describe("the second side panel", () => {
  it("is drawn beside the conversation while the space has something to show, and not otherwise", () => {
    let open: string | null = null;
    const { app, container, frame, aside } = drawn(() =>
      open ? { key: `x:${open}`, title: () => `Item ${open}`, render: () => <p data-testid="side-body">{open}</p> } : null,
    );
    expect(aside()).toBeNull();
    expect(frame().hasAttribute("data-secondary")).toBe(false);
    open = "a";
    act(() => app.redraw());
    expect(frame().dataset.secondary).toBe("open");
    expect(aside()!.dataset.key).toBe("x:a");
    expect(container.querySelector('[data-testid="secondary-title"]')!.textContent).toBe("Item a");
    expect(container.querySelector('[data-testid="side-body"]')!.textContent).toBe("a");
    // In the centre cell, after the conversation: the right column stays where it was.
    expect(container.querySelector("#centre > #main")!.nextElementSibling).toBe(aside());
    expect(container.querySelector("#centre")!.nextElementSibling!.id).toBe("details");
  });

  it("closing it hands it back to the space, which lets go of what it showed", () => {
    let open: string | null = "a";
    let closed = 0;
    const { container, aside } = drawn(() =>
      open
        ? {
            key: open,
            title: () => open!,
            render: () => null,
            onClose: () => {
              closed++;
              open = null;
            },
          }
        : null,
    );
    expect(aside()).not.toBeNull();
    act(() => fireEvent.click(container.querySelector('[data-testid="secondary-close"]')!));
    expect(closed).toBe(1);
    expect(aside()).toBeNull();
  });

  it("has a width of its own: the space's within bounds, then what the reader dragged, remembered", () => {
    const { app, frame } = drawn(() => ({ key: "k", title: () => "t", render: () => null, width: 9999 }));
    expect(frame().style.getPropertyValue("--secondary-width")).toBe(`${WIDTHS.secondary[2]}px`);
    expect(secondaryWidthOf({ dragged: {} }, undefined)).toBe(WIDTHS.secondary[1]);
    expect(secondaryWidthOf({ dragged: {} }, 10)).toBe(WIDTHS.secondary[0]);
    act(() => app.setWidth("secondary", 500));
    expect(frame().style.getPropertyValue("--secondary-width")).toBe("500px");
    expect(JSON.parse(localStorage.getItem("harnix.widths")!)).toMatchObject({ secondary: 500 });
    act(() => app.setWidth("secondary", null));
    expect(frame().style.getPropertyValue("--secondary-width")).toBe(`${WIDTHS.secondary[2]}px`);
  });

  it("maximized it takes the whole centre; restored it is back at its width; per space, remembered", () => {
    const { app, container, frame } = drawn(() => ({ key: "k", title: () => "t", render: () => null, width: 520 }));
    const button = () => container.querySelector<HTMLButtonElement>('[data-testid="secondary-max"]')!;
    // Left of the close button.
    expect(button().nextElementSibling!.getAttribute("data-testid")).toBe("secondary-close");
    expect(button().getAttribute("aria-pressed")).toBe("false");
    expect(button().title).toBe("Maximize the details: the conversation makes way");
    expect(frame().hasAttribute("data-secondary-max")).toBe(false);
    act(() => app.toggleDetailsMax(true));
    act(() => fireEvent.click(button()));
    expect(frame().hasAttribute("data-secondary-max")).toBe(true);
    expect(app.snapshot.detailsMax).toBe(false);
    expect(button().getAttribute("aria-pressed")).toBe("true");
    expect(button().title).toBe("Restore the details beside the conversation");
    expect(JSON.parse(localStorage.getItem("harnix.secondaryMax")!)).toEqual({ side: true });
    // A redraw keeps it; another space has its own.
    act(() => app.redraw());
    expect(frame().hasAttribute("data-secondary-max")).toBe(true);
    act(() => app.set({ space: "b" }));
    expect(app.secondaryMax()).toBe(false);
    act(() => app.set({ space: "side" }));
    expect(app.secondaryMax()).toBe(true);
    // Restored: the width it had.
    act(() => fireEvent.click(button()));
    expect(frame().hasAttribute("data-secondary-max")).toBe(false);
    expect(frame().style.getPropertyValue("--secondary-width")).toBe("520px");
    expect(JSON.parse(localStorage.getItem("harnix.secondaryMax")!)).toEqual({});
  });

  it("a space whose panel throws draws none", () => {
    const { aside } = drawn(() => {
      throw new Error("no");
    });
    expect(aside()).toBeNull();
  });
});
