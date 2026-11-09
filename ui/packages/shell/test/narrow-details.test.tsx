/**
 * Narrow windows. `[shell] narrow_details` (ipdrun-50): the frame says which narrow layout the
 * profile asked for; the stylesheet does the rest. The sidebar's state at 960px and below is the
 * shell's to say (review G2): the rail, or the drawer the person opened, over a wide choice kept.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import type { HarnixClient, Hello } from "@harnix/core";
import { HarnixUiProvider } from "@harnix/react";

import { Shell } from "../src/Shell";
import { scriptedApp } from "./scripted";

afterEach(cleanup);

function frameOf(shell: Hello["shell"]): HTMLElement {
  const { app } = scriptedApp();
  app.set({ hello: { shell } as unknown as Hello });
  const { container } = render(
    <HarnixUiProvider client={app.client as HarnixClient}>
      <Shell app={app} />
    </HarnixUiProvider>,
  );
  return container.querySelector("#frame") as HTMLElement;
}

describe("the profile's narrow_details", () => {
  it("absent or over: no mark, the right column goes over the conversation as before", () => {
    expect(frameOf({}).hasAttribute("data-narrow")).toBe(false);
    cleanup();
    expect(frameOf({ narrow_details: "over" }).hasAttribute("data-narrow")).toBe(false);
  });

  it("beside: the frame is marked, and the app says so", () => {
    expect(frameOf({ narrow_details: "beside" }).getAttribute("data-narrow")).toBe("beside");
    const { app } = scriptedApp();
    expect(app.narrowDetails()).toBe("over");
    app.set({ hello: { shell: { narrow_details: "beside" } } as unknown as Hello });
    expect(app.narrowDetails()).toBe("beside");
  });
});

describe("the sidebar in a narrow window", () => {
  it("is the rail; the toggle opens and closes a drawer; the wide choice stays for when it widens", () => {
    const { app } = scriptedApp();
    expect(app.sidebarShown()).toBe("expanded");
    app.toggleSidebar();
    expect(app.sidebarShown()).toBe("collapsed");
    app.toggleSidebar();
    app.setNarrow(true);
    // Narrow: the rail, whatever the wide choice.
    expect(app.sidebarShown()).toBe("collapsed");
    app.toggleSidebar();
    expect(app.snapshot.drawer).toBe(true);
    expect(app.sidebarShown()).toBe("expanded");
    app.toggleSidebar();
    expect(app.sidebarShown()).toBe("collapsed");
    // Wider again: the drawer goes, the wide choice is back.
    app.toggleSidebar();
    app.setNarrow(false);
    expect(app.snapshot.drawer).toBe(false);
    expect(app.sidebarShown()).toBe("expanded");
  });

  it("draws the frame as the app says, and a space picked in the drawer closes it", async () => {
    const { app } = scriptedApp();
    const { container } = render(
      <HarnixUiProvider client={app.client as HarnixClient}>
        <Shell app={app} />
      </HarnixUiProvider>,
    );
    const frame = container.querySelector("#frame") as HTMLElement;
    act(() => app.setNarrow(true));
    expect(frame.dataset.sidebar).toBe("collapsed");
    expect(frame.hasAttribute("data-drawer")).toBe(false);
    act(() => app.toggleSidebar());
    expect(frame.dataset.sidebar).toBe("expanded");
    expect(frame.hasAttribute("data-drawer")).toBe(true);
    expect(container.querySelector('[data-testid="drawer-veil"]')).not.toBeNull();
    await act(() => app.switchSpace("b"));
    expect(frame.dataset.sidebar).toBe("collapsed");
    expect(container.querySelector('[data-testid="drawer-veil"]')).toBeNull();
    // The veil closes it too.
    act(() => app.toggleSidebar());
    fireEvent.click(container.querySelector('[data-testid="drawer-veil"]')!);
    expect(app.snapshot.drawer).toBe(false);
  });
});
