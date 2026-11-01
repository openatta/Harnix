/** `host.shell.badge` (ipdrun-33): a count in front of the page title, kept by the shell. */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider } from "@harnix/react";

import { titleOf } from "../src/brand";
import { Shell } from "../src/Shell";
import { hostFor } from "../src/host";
import { scriptedApp } from "./scripted";

afterEach(cleanup);

describe("title badge", () => {
  it("puts the count in front of the title the shell writes, and takes it off again", () => {
    const { app } = scriptedApp();
    render(
      <HarnixUiProvider client={app.client as HarnixClient}>
        <Shell app={app} />
      </HarnixUiProvider>,
    );
    const plain = document.title;
    expect(plain).not.toMatch(/^\(\d+\) /);
    const host = hostFor(app);
    act(() => host.shell.badge(3));
    expect(document.title).toBe(`(3) ${plain}`);
    act(() => host.shell.badge(0));
    expect(document.title).toBe(plain);
    act(() => host.shell.badge(2));
    act(() => host.shell.badge(null));
    expect(document.title).toBe(plain);
  });

  it("only a whole number above 0 counts", () => {
    const { app } = scriptedApp();
    const host = hostFor(app);
    for (const n of [-1, Number.NaN, 0]) {
      host.shell.badge(n);
      expect(app.snapshot.titleBadge).toBeNull();
    }
    host.shell.badge(4.7);
    expect(app.snapshot.titleBadge).toBe(4);
    expect(titleOf("Harnix", 4)).toBe("(4) Harnix");
    expect(titleOf("Harnix", null)).toBe("Harnix");
  });
});
