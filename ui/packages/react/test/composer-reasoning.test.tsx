/** 模型菜单里的推理强度与思考开关：照会话说的标亮当前值，改的只送改了的那一个，`null` 是回到设置的值。 */

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { ChatComposer, HarnixUiProvider, en, setLocale, type ReasoningChange } from "../src";
import { FakeClient } from "./fake-client";

afterEach(cleanup);
setLocale("en");

function mount(props: { reasoning?: Parameters<typeof ChatComposer>[0]["reasoning"]; onReasoning?: (change: ReasoningChange) => Promise<void> | void }) {
  const server = new FakeClient();
  return render(
    <HarnixUiProvider client={server.asClient()} strings={en}>
      <ChatComposer
        state={null}
        send={async () => true}
        interrupt={() => undefined}
        sendMode="stop"
        disabled={false}
        placeholder=""
        quickReplies={[]}
        commands={null}
        contextRing={null}
        options={null}
        attachments={false}
        voice={false}
        model="m"
        models={[{ id: "m" }]}
        onModel={() => undefined}
        reasoning={props.reasoning}
        onReasoning={props.onReasoning}
      />
    </HarnixUiProvider>,
  );
}

describe("a session's reasoning in the model menu", () => {
  it("shows the session's own effort and the settings' thinking, and sends only what changed", async () => {
    const changes: ReasoningChange[] = [];
    mount({ reasoning: { effort: "high", effort_source: "session", thinking_mode: "auto", thinking_mode_source: "settings" }, onReasoning: (c) => void changes.push(c) });
    fireEvent.click(screen.getByTestId("conv-model"));
    const effort = await screen.findByTestId("conv-effort");
    expect(effort.getAttribute("data-current")).toBe("high");
    expect(screen.getByTestId("conv-effort-high").getAttribute("aria-pressed")).toBe("true");
    // Thinking is the settings': "as set", with what that is.
    expect(screen.getByTestId("conv-thinking_mode-default").getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByTestId("conv-thinking_mode-default").textContent).toBe("As set (auto)");
    fireEvent.click(screen.getByTestId("conv-thinking_mode-off"));
    await waitFor(() => expect(changes).toEqual([{ thinking_mode: "off" }]));
    fireEvent.click(screen.getByTestId("conv-effort-default"));
    await waitFor(() => expect(changes).toEqual([{ thinking_mode: "off" }, { effort: null }]));
    // What is already chosen is not sent again.
    fireEvent.click(screen.getByTestId("conv-effort-high"));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(changes.length).toBe(2);
  });

  it("draws no rows when the session's reasoning can be neither shown nor changed", async () => {
    mount({});
    fireEvent.click(screen.getByTestId("conv-model"));
    await screen.findByTestId("conv-model-menu");
    expect(screen.queryByTestId("conv-effort")).toBeNull();
  });
});
