/** `[shell] new_session = false` (ipdrun-46): no "New session" button, and ⌘K is not the shell's. */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import type { HarnixClient, Hello } from "@harnix/core";
import { HarnixUiProvider } from "@harnix/react";

import { Shell } from "../src/Shell";
import { registerShellShortcuts } from "../src/shortcuts";
import { scriptedApp, space } from "./scripted";

afterEach(cleanup);

function hello(shell: Partial<Hello["shell"]>): Hello {
  return { shell } as unknown as Hello;
}

function draw(app: ReturnType<typeof scriptedApp>["app"]) {
  registerShellShortcuts(app);
  return render(
    <HarnixUiProvider client={app.conn as HarnixClient}>
      <Shell app={app} />
    </HarnixUiProvider>,
  );
}

/** ⌘K at the window (Ctrl+K: the test page is not a Mac); whether the shell took it. */
function commandK(): boolean {
  const event = new KeyboardEvent("keydown", { key: "k", ctrlKey: true, bubbles: true, cancelable: true });
  act(() => {
    window.dispatchEvent(event);
  });
  return event.defaultPrevented;
}

describe("the profile's new_session switch", () => {
  it("absent or true: the button and ⌘K are there, as before", () => {
    for (const shell of [{}, { new_session: true }]) {
      const { app } = scriptedApp();
      app.set({ hello: hello(shell) });
      const { container, unmount } = draw(app);
      expect(container.querySelector("#new")).not.toBeNull();
      expect(container.querySelector("#new + .sep")).not.toBeNull();
      expect(app.startsSessions()).toBe(true);
      expect(commandK()).toBe(true);
      unmount();
    }
  });

  it("false: no button, no separator after it, and ⌘K does nothing", () => {
    const { app } = scriptedApp();
    app.set({ hello: hello({ new_session: false }) });
    const { container } = draw(app);
    expect(container.querySelector("#new")).toBeNull();
    expect(container.querySelector("#side > .sep:not(.chats-sep)")).toBeNull();
    expect(app.startsSessions()).toBe(false);
    const current = app.snapshot.current;
    expect(commandK()).toBe(false);
    expect(app.snapshot.current).toBe(current);
  });

  it("a space's own newSession still applies on top", () => {
    const { app } = scriptedApp();
    app.registry.register("space", space("quiet", { conversation: { newSession: false } }), "test");
    app.set({ hello: hello({ new_session: true }) });
    const { container } = draw(app);
    expect(container.querySelector("#new")).not.toBeNull();
    act(() => app.set({ space: "quiet" }));
    expect(container.querySelector("#new")).toBeNull();
  });
});
