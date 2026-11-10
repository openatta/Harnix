/**
 * A space shown inside another (`within`, as the flows are inside the system space): not listed,
 * the other's entry lit while it is in front, never landed on by itself. Where the flows go is
 * `@harnix/flow`'s own test.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider } from "@harnix/react";

import type { App } from "../src/app";
import { Shell } from "../src/Shell";
import { scriptedApp, space, tick } from "./scripted";

afterEach(cleanup);

/** The scripted spaces, and one at the bottom that has no conversation (as the system space). */
function withSystem() {
  const { app, client } = scriptedApp();
  client.answer = (method) => (method === "harnix.sessions.list" ? { sessions: [] } : {});
  app.registry.register("space", space("system", { order: 90, place: "bottom", center: { mode: "hidden", overview: () => null } }), "test");
  return { app, client };
}

function draw(app: App) {
  return render(
    <HarnixUiProvider client={app.conn as HarnixClient}>
      <Shell app={app} />
    </HarnixUiProvider>,
  );
}

describe("a space shown inside another", () => {
  /** The scripted spaces, the system space, and one shown inside it. */
  function nested() {
    const { app } = withSystem();
    app.registry.register("space", space("inner", { order: 5, within: "system" }), "test");
    return app;
  }

  it("is not listed; in front, the other's entry is lit", async () => {
    const app = nested();
    const { container } = draw(app);
    await act(tick);
    expect(container.querySelector('#side button[data-space="inner"]')).toBeNull();
    const system = () => container.querySelector('#side button[data-space="system"]')!;
    expect(system().getAttribute("aria-pressed")).toBe("false");
    await act(() => app.switchSpace("inner"));
    expect(container.querySelector("#main")?.getAttribute("data-space")).toBe("inner");
    expect(system().getAttribute("aria-pressed")).toBe("true");
    expect(container.querySelector('#side .spaces [aria-pressed="true"]')).toBeNull();
  });

  it("the new-session button from inside it goes to a space with conversations of its own", async () => {
    const app = nested();
    const { container } = draw(app);
    await act(tick);
    await act(() => app.switchSpace("inner"));
    await act(async () => {
      container.querySelector<HTMLButtonElement>("#new")!.click();
      await tick();
    });
    expect(app.snapshot.space).toBe("sessions");
  });

  it("landing, the stored space gone, never picks it by itself", async () => {
    const app = nested();
    app.set({ space: "gone" });
    (app as unknown as { landOnASpace(): void }).landOnASpace();
    // First by order is the one inside (5), then the system space (90): the system space.
    expect(app.snapshot.space).toBe("system");
  });
});
