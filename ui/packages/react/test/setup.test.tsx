/**
 * A first-run step: drawn while it is to do; once done, drawn as done only if this page saw it to
 * do (a step done before is not drawn at all, nor one not known yet); closed, never again in this
 * browser.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";

import { SetupStep, resetSetupSteps, setupStepState, useSetupStep } from "../src/setup";

afterEach(() => {
  cleanup();
  localStorage.clear();
  resetSetupSteps();
});

function Step({ done }: { done: boolean | null }) {
  const state = useSetupStep("x.step", done);
  return (
    <SetupStep id="x.step" state={state} icon="key" title="Step">
      body
    </SetupStep>
  );
}

describe("a first-run step", () => {
  it("is hidden until known, to do while not done, done once done after that", () => {
    expect(setupStepState("a", null)).toBe("hidden");
    expect(setupStepState("a", true)).toBe("hidden");
    expect(setupStepState("b", false)).toBe("todo");
    expect(setupStepState("b", true)).toBe("done");
  });

  it("draws its state, and once closed draws nothing, for good", () => {
    const page = render(<Step done={false} />);
    const step = () => page.container.querySelector('[data-step="x.step"]') as HTMLElement | null;
    expect(step()?.dataset.state).toBe("todo");
    page.rerender(<Step done={true} />);
    expect(step()?.dataset.state).toBe("done");
    act(() => fireEvent.click(page.getByTestId("setup-dismiss")));
    expect(page.container.innerHTML).toBe("");
    expect(JSON.parse(localStorage.getItem("harnix.setup.dismissed")!)).toEqual(["x.step"]);
    resetSetupSteps();
    expect(setupStepState("x.step", false)).toBe("hidden");
  });
});
