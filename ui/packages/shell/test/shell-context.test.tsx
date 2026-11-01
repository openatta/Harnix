/** `host.shell.context` (ipdrun-49): the application's line under the brand in the left column. */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider } from "@harnix/react";

import { Shell } from "../src/Shell";
import { hostFor } from "../src/host";
import { scriptedApp, tick } from "./scripted";

afterEach(cleanup);

function drawn() {
  const { app } = scriptedApp();
  const { container } = render(
    <HarnixUiProvider client={app.client as HarnixClient}>
      <Shell app={app} />
    </HarnixUiProvider>,
  );
  const line = () => container.querySelector<HTMLElement>('[data-testid="shell-context"]');
  return { app, host: hostFor(app), container, line };
}

describe("shell context line", () => {
  it("is not drawn until asked for", () => {
    const { line } = drawn();
    expect(line()).toBeNull();
  });

  it("draws under the brand: label, tooltip, tone, a dot when active; plain text without onClick", () => {
    const { host, container, line } = drawn();
    act(() => host.shell.context({ label: "Rocket", title: "/work/rocket", tone: "active" }));
    expect(line()!.textContent).toBe("Rocket");
    expect(line()!.title).toBe("/work/rocket");
    expect(line()!.dataset.tone).toBe("active");
    expect(line()!.querySelector(".dot")).not.toBeNull();
    expect(line()!.tagName).toBe("DIV");
    // Right under the brand's row.
    expect(container.querySelector("#side .l-top")!.nextElementSibling).toBe(line());
  });

  it("is a button when it has onClick; muted has no dot; null takes it away", () => {
    const { host, line } = drawn();
    let clicks = 0;
    act(() => host.shell.context({ label: "No open project", tone: "muted", onClick: () => clicks++ }));
    expect(line()!.tagName).toBe("BUTTON");
    expect(line()!.dataset.tone).toBe("muted");
    expect(line()!.querySelector(".dot")).toBeNull();
    fireEvent.click(line()!);
    expect(clicks).toBe(1);
    act(() => host.shell.context(null));
    expect(line()).toBeNull();
    act(() => host.shell.context({ label: "" }));
    expect(line()).toBeNull();
  });

  it("stays across a space switch", async () => {
    const { app, host, line } = drawn();
    act(() => host.shell.context({ label: "Rocket" }));
    await act(async () => {
      await app.switchSpace("b");
      await tick();
    });
    expect(app.snapshot.space).toBe("b");
    expect(line()!.textContent).toBe("Rocket");
    expect(line()!.dataset.tone).toBeUndefined();
  });
});
