/** 输入框的权限模式：按钮显示会话此刻的模式，菜单换模式只送选了的那一个；被设置夹窄时按钮和菜单都说出来；新会话可以随设置。 */

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { ChatComposer, HarnixUiProvider, en, setLocale } from "../src";
import { FakeClient } from "./fake-client";

afterEach(cleanup);
setLocale("en");

type Props = Parameters<typeof ChatComposer>[0];

function mount(props: Pick<Props, "permissionMode" | "onPermissionMode" | "permissionFollows" | "permissionNote">) {
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
        {...props}
      />
    </HarnixUiProvider>,
  );
}

describe("the permission mode in the composer", () => {
  it("shows the mode the session is in and sends only a change", async () => {
    const asked: (string | null)[] = [];
    mount({ permissionMode: "plan", onPermissionMode: (m) => void asked.push(m) });
    const button = screen.getByTestId("conv-permission");
    expect(button.getAttribute("data-mode")).toBe("plan");
    expect(button.textContent).toContain("Plan: read only");
    expect(button.getAttribute("aria-label")).toBe("Permission mode: Plan: read only");
    fireEvent.click(button);
    await screen.findByTestId("conv-permission-menu");
    expect(screen.getByTestId("conv-permission-note").textContent).toBe(en.permissionFoot);
    expect(screen.queryByTestId("conv-permission-settings")).toBeNull();
    // The mode it is in: nothing sent, the menu closes.
    fireEvent.click(screen.getByTestId("conv-permission-plan"));
    await waitFor(() => expect(screen.queryByTestId("conv-permission-menu")).toBeNull());
    expect(asked).toEqual([]);
    fireEvent.click(button);
    fireEvent.click(await screen.findByTestId("conv-permission-acceptEdits"));
    await waitFor(() => expect(asked).toEqual(["acceptEdits"]));
    await waitFor(() => expect(screen.queryByTestId("conv-permission-menu")).toBeNull());
  });

  it("says when the settings held the session narrower", async () => {
    mount({ permissionMode: "acceptEdits", onPermissionMode: () => undefined, permissionNote: "held to Accept edits" });
    const button = screen.getByTestId("conv-permission");
    expect(button.getAttribute("data-clamped")).toBe("true");
    fireEvent.click(button);
    expect((await screen.findByTestId("conv-permission-note")).textContent).toBe("held to Accept edits");
  });

  it("offers a new session the settings' mode first, and a mode the engine added keeps its name", async () => {
    const asked: (string | null)[] = [];
    const { unmount } = mount({ permissionMode: null, permissionFollows: true, onPermissionMode: (m) => void asked.push(m) });
    expect(screen.getByTestId("conv-permission").textContent).toContain("As set");
    fireEvent.click(screen.getByTestId("conv-permission"));
    expect((await screen.findByTestId("conv-permission-settings")).textContent).toContain("As set");
    expect(screen.getByTestId("conv-permission-note").textContent).toBe(en.permissionFootNew);
    fireEvent.click(screen.getByTestId("conv-permission-dontAsk"));
    await waitFor(() => expect(asked).toEqual(["dontAsk"]));
    unmount();

    mount({ permissionMode: "auto", onPermissionMode: () => undefined });
    fireEvent.click(screen.getByTestId("conv-permission"));
    expect((await screen.findByTestId("conv-permission-auto")).textContent).toContain("auto");
  });

  it("draws no control when the mode can be neither shown nor changed", () => {
    mount({});
    expect(screen.queryByTestId("conv-permission")).toBeNull();
  });
});
