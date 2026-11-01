/** The left column's space list: groups (ipdrun-47) and spaces that cannot be entered for now (ipdrun-48). */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider } from "@harnix/react";

import type { App } from "../src/app";
import { Shell } from "../src/Shell";
import { hostFor } from "../src/host";
import { scriptedApp, space, tick } from "./scripted";

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

/** `landOnASpace` runs once the page is connected; here it is called the way `boot` does. */
function land(app: App): void {
  (app as unknown as { landOnASpace(): void }).landOnASpace();
}

describe("a disabled space", () => {
  it("is greyed, says why on hover, and a click does not switch; redraw brings it back", async () => {
    const { app } = scriptedApp();
    let why: string | false = "Open a project first";
    app.registry.register("space", space("work", { disabled: () => why }), "test");
    const { container } = draw(app);
    const entry = () => container.querySelector<HTMLButtonElement>('nav.spaces [data-space="work"]')!;
    expect(entry().classList.contains("disabled")).toBe(true);
    expect(entry().getAttribute("aria-disabled")).toBe("true");
    expect(entry().hasAttribute("data-disabled")).toBe(true);
    expect(entry().title).toBe("Open a project first");
    fireEvent.click(entry());
    await act(tick);
    expect(app.snapshot.space).toBe("sessions");

    why = false;
    act(() => hostFor(app).redraw());
    expect(entry().classList.contains("disabled")).toBe(false);
    expect(entry().hasAttribute("aria-disabled")).toBe(false);
    expect(entry().title).toBe("work");
    fireEvent.click(entry());
    await act(tick);
    expect(app.snapshot.space).toBe("work");
  });

  it("host.spaces.switch to it settles without switching; others switch as before", async () => {
    const { app } = scriptedApp();
    app.registry.register("space", space("work", { disabled: () => "not now" }), "test");
    const host = hostFor(app);
    await host.spaces.switch("work");
    expect(host.spaces.current()).toBe("sessions");
    await host.spaces.switch("b");
    expect(host.spaces.current()).toBe("b");
    await expect(host.spaces.switch("nope")).rejects.toThrow();
  });

  it("a throwing disabled() counts as enabled, and is noted", async () => {
    const { app } = scriptedApp();
    app.registry.register("space", space("work", { disabled: () => { throw new Error("disabled broke"); } }), "test");
    const { container } = draw(app);
    const entry = container.querySelector<HTMLButtonElement>('nav.spaces [data-space="work"]')!;
    expect(entry.classList.contains("disabled")).toBe(false);
    await act(tick);
    expect(app.registry.refusals().map((r) => r.reason)).toContain("disabled broke");
    fireEvent.click(entry);
    await act(tick);
    expect(app.snapshot.space).toBe("work");
  });

  it("landing: a stored space that is disabled gives way to the first that is not, by order", () => {
    const { app } = scriptedApp();
    app.registry.register("space", space("first", { order: 1, disabled: () => "closed" }), "test");
    app.registry.register("space", space("second", { order: 2 }), "test");
    app.registry.register("space", space("held", { order: 3, disabled: () => "closed" }), "test");
    app.set({ space: "held" });
    land(app);
    expect(app.snapshot.space).toBe("second");

    app.set({ space: "b" }); // enabled: stays
    land(app);
    expect(app.snapshot.space).toBe("b");

    app.set({ space: "gone" }); // not registered: the first enabled, not merely the first
    land(app);
    expect(app.snapshot.space).toBe("second");
  });

  it("landing without any disabled() is as before: a missing stored space goes to the first by order", () => {
    const { app } = scriptedApp();
    app.registry.register("space", space("first", { order: 1 }), "test");
    app.set({ space: "gone" });
    land(app);
    expect(app.snapshot.space).toBe("first");
  });
});
