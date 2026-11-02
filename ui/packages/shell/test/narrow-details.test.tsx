/** `[shell] narrow_details` (ipdrun-50): the frame says which narrow layout the profile asked for; the stylesheet does the rest. */

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render } from "@testing-library/react";
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
