/**
 * The display preferences: kept and checked by the display module, read by the chat components at
 * their next draw — a tool's details, a finished thought, the usage pill, a link in a message.
 */

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Block } from "@harnix/core/state";
import { ChatPanel, DISPLAY_DEFAULTS, HarnixUiProvider, TranscriptList, applyTypography, displayAccepts, display, setDisplay } from "../src";
import { FakeClient, attached, flush } from "./fake-client";

let server: FakeClient;

beforeEach(() => {
  localStorage.clear();
  setDisplay(DISPLAY_DEFAULTS);
  server = new FakeClient();
  server.handlers["harnix.session.attach"] = (p) => attached(String(p.session_id));
});

afterEach(() => {
  cleanup();
  setDisplay(DISPLAY_DEFAULTS);
});

const BLOCKS: Block[] = [
  { key: 1, kind: "think", text: "weighing it", startedAt: 0, endedAt: 2000 },
  { key: 2, kind: "tool", id: "t1", name: "Read", input: { file_path: "/a.txt" }, result: "hello", status: "ok" },
];

function list() {
  return render(
    <HarnixUiProvider client={server.asClient()}>
      <TranscriptList blocks={BLOCKS} />
    </HarnixUiProvider>,
  );
}

describe("the display module", () => {
  it("keeps what it accepts, ignores what it does not, and stores only what differs from the default", () => {
    setDisplay({ fontSize: 16, links: "ask", enter: "steer" });
    expect(display()).toMatchObject({ fontSize: 16, links: "ask", enter: "steer" });
    expect(JSON.parse(localStorage.getItem("harnix.display")!)).toEqual({ fontSize: 16, links: "ask", enter: "steer" });
    setDisplay({ fontSize: 99, links: "elsewhere" as never, usage: "full" });
    expect(display()).toMatchObject({ fontSize: 16, links: "ask" });
    setDisplay(DISPLAY_DEFAULTS);
    expect(localStorage.getItem("harnix.display")).toBeNull();
  });

  it("refuses a font list that could end a declaration", () => {
    expect(displayAccepts("fontBody", '"Inter", sans-serif')).toBe(true);
    expect(displayAccepts("fontBody", "x; background: url(//e.x)")).toBe(false);
    expect(displayAccepts("fontCode", "a } b")).toBe(false);
    expect(displayAccepts("fontSize", 14.5)).toBe(false);
  });

  it("puts the type scale and the families on the document's tokens, live, and takes them off at the default", () => {
    const root = document.documentElement;
    setDisplay({ fontSize: 17, fontBody: "Georgia, serif", fontCode: "Menlo" });
    expect(root.style.getPropertyValue("--font-size-base")).toBe("17px");
    expect(root.style.getPropertyValue("--font-sans")).toBe("Georgia, serif");
    expect(root.style.getPropertyValue("--font-code")).toBe("Menlo");
    setDisplay({ fontSize: 14, fontBody: "", fontCode: "" });
    expect(root.style.getPropertyValue("--font-size-base")).toBe("");
    expect(root.style.getPropertyValue("--font-sans")).toBe("");
    applyTypography(display(), null);
  });
});

describe("what a conversation draws", () => {
  it("folds a finished thought and a tool's details by default, and opens them when asked to", () => {
    const first = list();
    expect(screen.queryByTestId("conv-think-text")).toBeNull();
    expect(screen.getByTestId("conv-tool").getAttribute("aria-expanded")).toBe("false");
    first.unmount();
    setDisplay({ thinkingFolded: false, toolsOpen: true });
    list();
    expect(screen.getByTestId("conv-think-text").textContent).toContain("weighing it");
    expect(screen.getByTestId("conv-tool").getAttribute("aria-expanded")).toBe("true");
  });

  function mountWithReply() {
    render(
      <HarnixUiProvider client={server.asClient()}>
        <ChatPanel sessionId="S1" header={{ usage: true }} />
      </HarnixUiProvider>,
    );
    return act(async () => {
      await flush();
      server.event("S1", { kind: "text_delta", text: "see [docs](https://e.x/d)" });
      server.event("S1", { kind: "turn_complete", stop_reason: "end_turn", usage: { input_tokens: 12_000, output_tokens: 345, cache_read_input_tokens: 5000, cache_creation_input_tokens: 0 } });
    });
  }

  it("says the tokens in full, in short, or not at all", async () => {
    await mountWithReply();
    const pill = () => screen.queryByTestId("conv-usage");
    expect(pill()!.dataset.detail).toBe("full");
    expect(pill()!.textContent).toContain("17,345");
    expect(pill()!.dataset.cached).toBe("5000");
    act(() => setDisplay({ usage: "compact" }));
    expect(pill()!.textContent).toContain("17k");
    expect(pill()!.textContent).not.toContain("·");
    act(() => setDisplay({ usage: "none" }));
    expect(pill()).toBeNull();
  });

  it("opens a link in this tab, after asking, or as the link says", async () => {
    const open = vi.spyOn(window, "open").mockImplementation(() => null);
    const confirm = vi.spyOn(window, "confirm");
    await mountWithReply();
    const link = () => screen.getByText("docs");
    // New tab: the link's own target; the panel does not step in.
    expect(fireEvent.click(link())).toBe(true);
    expect(open).not.toHaveBeenCalled();
    act(() => setDisplay({ links: "same" }));
    expect(fireEvent.click(link())).toBe(false);
    expect(open).toHaveBeenLastCalledWith("https://e.x/d", "_self", "noopener");
    act(() => setDisplay({ links: "ask" }));
    confirm.mockReturnValueOnce(false);
    fireEvent.click(link());
    expect(open).toHaveBeenCalledTimes(1);
    confirm.mockReturnValueOnce(true);
    fireEvent.click(link());
    expect(open).toHaveBeenLastCalledWith("https://e.x/d", "_blank", "noopener,noreferrer");
    open.mockRestore();
    confirm.mockRestore();
  });
});
