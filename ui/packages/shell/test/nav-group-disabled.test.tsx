/** The left column's space list: groups (ipdrun-47). */

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider } from "@harnix/react";

import type { App } from "../src/app";
import { Shell } from "../src/Shell";
import { scriptedApp, space } from "./scripted";

afterEach(cleanup);

function draw(app: App) {
  return render(
    <HarnixUiProvider client={app.client as HarnixClient}>
      <Shell app={app} />
    </HarnixUiProvider>,
  );
}

/** The space list as drawn: space ids, and "|" for each separator. */
function listed(container: HTMLElement): string[] {
  return [...container.querySelectorAll("nav.spaces > *")].map((el) => (el.getAttribute("data-testid") === "space-sep" ? "|" : el.getAttribute("data-space") ?? "?"));
}

describe("space groups", () => {
  it("no space names a group: no separator", () => {
    const { app } = scriptedApp();
    app.registry.register("space", space("c"), "test");
    const { container } = draw(app);
    expect(container.querySelectorAll('[data-testid="space-sep"]')).toHaveLength(0);
    expect(listed(container)).toEqual(["sessions", "b", "c"]);
  });

  it("a line between two neighbours whose groups differ, unset counting as \"\"", () => {
    const { app } = scriptedApp();
    app.registry.register("space", space("c", { group: "work", order: 200 }), "test");
    app.registry.register("space", space("d", { group: "work", order: 201 }), "test");
    app.registry.register("space", space("e", { group: "", order: 300 }), "test");
    const { container } = draw(app);
    expect(listed(container)).toEqual(["sessions", "b", "|", "c", "d", "|", "e"]);
    expect(container.querySelector('[data-testid="space-sep"]')?.getAttribute("role")).toBe("separator");
  });
});
