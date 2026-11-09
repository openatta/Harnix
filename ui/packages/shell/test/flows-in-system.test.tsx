/**
 * The flows inside the system space: with the system space, the flow module's interface space is
 * shown inside it (`within`) — not listed, the system entry lit while it is in front — and the
 * console is handed its Flows tab. Without the system space it is the flow space of its own.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider } from "@harnix/react";

import type { App } from "../src/app";
import { registerFlow } from "../src/flow";
import { Shell } from "../src/Shell";
import { systemFlows } from "../src/system-flows";
import { scriptedApp, space, tick } from "./scripted";

afterEach(cleanup);

const LIST = { spaces: [{ id: "sessions", scene: null }, { id: "flow", scene: "flow-author" }, { id: "system", scene: null }] };

function withFlowModule(system: boolean) {
  const { app, client } = scriptedApp();
  client.answer = (method) => (method === "harnix.spaces.list" ? LIST : method === "harnix.sessions" ? { sessions: [] } : {});
  app.set({ reachable: ["app.flow.defs", "harnix.spaces.list"] });
  if (system) app.registry.register("space", space("system", { order: 90, center: "hidden", overview: () => null }), "builtin");
  return { app, client };
}

function draw(app: App) {
  return render(
    <HarnixUiProvider client={app.client as HarnixClient}>
      <Shell app={app} />
    </HarnixUiProvider>,
  );
}

describe("where the flows go", () => {
  it("with the system space: inside it, the Flows tab handed to its console, the reference the one right tab", async () => {
    const { app } = withFlowModule(true);
    expect(systemFlows(app)).toBe(null);
    await registerFlow(app, () => true);
    const flow = app.registry.at("space").find((c) => c.id === "flow");
    expect(flow).toMatchObject({ within: "system", center: "chat", owner: "flowui" });
    expect(typeof flow?.sidebar).toBe("function");
    expect((flow?.tabs as { id: string }[]).map((x) => x.id)).toEqual(["flow.reference"]);
    expect(systemFlows(app)?.title()).toBe("Flows");
  });

  it("without it: the flow space of its own, as before, and no Flows tab", async () => {
    const { app } = withFlowModule(false);
    await registerFlow(app, () => true);
    const flow = app.registry.at("space").find((c) => c.id === "flow");
    expect(flow?.within).toBeUndefined();
    expect((flow?.tabs as { id: string }[]).map((x) => x.id)).toEqual(["flow.flows", "flow.runs", "flow.reference"]);
    expect(systemFlows(app)).toBe(null);
  });

  it("dropped by the profile: nowhere", async () => {
    const { app } = withFlowModule(true);
    await registerFlow(app, (id) => id !== "flow");
    expect(app.registry.at("space").some((c) => c.id === "flow")).toBe(false);
    expect(systemFlows(app)).toBe(null);
  });
});

describe("a space shown inside another", () => {
  /** The scripted spaces, the system space, and one shown inside it. */
  function nested() {
    const { app } = withFlowModule(true);
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
