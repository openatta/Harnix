/**
 * The general section's display preferences: rows of their own, written to the display module the
 * chat components read, applied at once, and put back to their defaults.
 */

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { DISPLAY_DEFAULTS, display, setDisplay } from "@harnix/react";

import { openSettings } from "../src/settings";
import { scriptedApp } from "./scripted";

beforeEach(() => {
  localStorage.clear();
  setDisplay(DISPLAY_DEFAULTS);
});

afterEach(() => {
  cleanup();
  setDisplay(DISPLAY_DEFAULTS);
});

/** Past the form reading its values again after a write: a second change made over the first's
 *  stale read is a conflict, as it is meant to be. */
const reread = () => new Promise((resolve) => setTimeout(resolve, 30));

async function general() {
  const { app } = scriptedApp();
  openSettings(app, "general");
  render(<>{app.snapshot.modal?.content}</>);
  await screen.findByTestId("settings-fontSize");
}

describe("display preferences", () => {
  it("draw a row each, with the preferences' defaults", async () => {
    await general();
    for (const key of ["fontSize", "fontBody", "fontCode", "toolsOpen", "thinkingFolded", "usage", "links", "enter"]) {
      expect(screen.getByTestId(`settings-${key}`), key).toBeTruthy();
    }
    expect((screen.getByTestId("settings-fontSize") as HTMLInputElement).value).toBe("14");
    expect((screen.getByTestId("settings-thinkingFolded") as HTMLInputElement).checked).toBe(true);
    expect((screen.getByTestId("settings-usage") as HTMLSelectElement).value).toBe("full");
    expect((screen.getByTestId("settings-enter") as HTMLSelectElement).value).toBe("queue");
  });

  it("apply when written: the module, the document's tokens", async () => {
    await general();
    fireEvent.change(screen.getByTestId("settings-links"), { target: { value: "ask" } });
    await waitFor(() => expect(display().links).toBe("ask"));
    await reread();
    fireEvent.change(screen.getByTestId("settings-enter"), { target: { value: "steer" } });
    await waitFor(() => expect(display().enter).toBe("steer"));
    await reread();
    fireEvent.click(screen.getByTestId("settings-toolsOpen"));
    await waitFor(() => expect(display().toolsOpen).toBe(true));
    await reread();
    const size = screen.getByTestId("settings-fontSize");
    fireEvent.change(size, { target: { value: "17" } });
    fireEvent.blur(size);
    await waitFor(() => expect(document.documentElement.style.getPropertyValue("--font-size-base")).toBe("17px"));
  });

  it("refuse a font size out of range at the row and keep the one there was", async () => {
    await general();
    const size = screen.getByTestId("settings-fontSize");
    fireEvent.change(size, { target: { value: "40" } });
    fireEvent.blur(size);
    expect(await screen.findByTestId("setting-problem")).toBeTruthy();
    expect(display().fontSize).toBe(14);
  });

  it("go back to the default from the row's button", async () => {
    setDisplay({ usage: "none" });
    await general();
    fireEvent.click(await screen.findByTestId("settings-usage-default"));
    await waitFor(() => expect(display().usage).toBe("full"));
  });
});
