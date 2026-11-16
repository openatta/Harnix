/**
 * The turn rail: one tick per turn from the session's outline and the turns shown, a preview on pointing or
 * focus, a jump that reads older history in first, the arrows inside it, ⌥↑ / ⌥↓ as `shortcut` commands, and
 * only the ticks in view mounted.
 */

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ChatPanel, HarnixUiProvider, Registry, registerTurnShortcuts, setLocale, type ChatPanelProps, type ShortcutCommand } from "../src";
import { FakeClient, attached, flush } from "./fake-client";

let server: FakeClient;

/** A transcript of `turns` turns, two messages each: turn `tN` asks `qN` and is answered `aN`. */
function transcript(turns: number) {
  return Array.from({ length: turns * 2 }, (_, seq) => {
    const n = Math.floor(seq / 2);
    return seq % 2 === 0
      ? { seq, role: "user", origin: "user", turn_id: `t${n}`, content: [{ type: "text", text: `q${n}` }] }
      : { seq, role: "assistant", origin: "assistant", turn_id: `t${n}`, content: [{ type: "text", text: `a${n} `.repeat(5) }] };
  });
}

/** The session holds `turns` turns, and the hub's outline lists them all. */
function serve(turns: number) {
  const log = transcript(turns);
  server.handlers["harnix.session.attach"] = (p) => attached(String(p.session_id), { history_total: log.length });
  server.handlers["engine.session.history"] = (p) => {
    const offset = Number(p.offset ?? 0);
    return { total: log.length, messages: log.slice(offset, offset + Number(p.limit ?? 100)) };
  };
  server.handlers["harnix.session.outline"] = () => ({
    instance: "main",
    session_id: "S1",
    total: log.length,
    turns: Array.from({ length: turns }, (_, n) => ({ turn_id: `t${n}`, seq: n * 2, at: "2026-11-16T08:00:00Z", prompt: `q${n}`, reply: `a${n}` })),
  });
}

function mount(props: Partial<ChatPanelProps> = {}) {
  return render(
    <HarnixUiProvider client={server.asClient()}>
      <ChatPanel sessionId="S1" {...props} />
    </HarnixUiProvider>,
  );
}

async function settle() {
  await act(async () => {
    await flush(10);
  });
}

const ticks = () => screen.queryAllByTestId("conv-turnrail-tick");
const tick = (index: number) => ticks().find((t) => t.dataset.index === String(index))!;

beforeEach(() => {
  setLocale("en");
  server = new FakeClient();
});

afterEach(() => cleanup());

describe("the turn rail", () => {
  it("has a tick per turn, the last one being read, and previews a turn on pointing or focus", async () => {
    serve(3);
    mount();
    await settle();
    const rail = screen.getByTestId("conv-turnrail");
    expect(rail.getAttribute("aria-label")).toBe("Turns");
    expect(ticks().map((t) => t.getAttribute("aria-label"))).toEqual(["Turn 1: q0", "Turn 2: q1", "Turn 3: q2"]);
    expect(tick(2).getAttribute("aria-current")).toBe("true");
    expect(tick(2).tabIndex).toBe(0);
    expect(tick(0).tabIndex).toBe(-1);
    // The turns' starts are where the rail lands.
    expect(document.querySelectorAll("[data-turn-anchor]")).toHaveLength(3);

    fireEvent.pointerEnter(tick(1));
    const preview = screen.getByTestId("conv-turnrail-preview");
    expect(preview.getAttribute("role")).toBe("tooltip");
    expect(screen.getByTestId("conv-turnrail-prompt").textContent).toBe("q1");
    expect(screen.getByTestId("conv-turnrail-reply").textContent).toBe("a1");
    expect(tick(1).getAttribute("aria-describedby")).toBe(preview.id);
    fireEvent.pointerLeave(rail.querySelector(".hx-turnrail-track")!);
    expect(screen.queryByTestId("conv-turnrail-preview")).toBeNull();

    act(() => tick(0).focus());
    expect(screen.getByTestId("conv-turnrail-prompt").textContent).toBe("q0");
    // The arrows move along the rail, and the preview with them.
    fireEvent.keyDown(tick(0), { key: "ArrowDown" });
    expect(document.activeElement).toBe(tick(1));
    expect(screen.getByTestId("conv-turnrail-prompt").textContent).toBe("q1");
    fireEvent.keyDown(tick(1), { key: "End" });
    expect(document.activeElement).toBe(tick(2));
    act(() => tick(2).blur());
    expect(screen.queryByTestId("conv-turnrail-preview")).toBeNull();
  });

  it("is not drawn for one turn, nor when the conversation turns it off", async () => {
    serve(1);
    mount();
    await settle();
    expect(screen.queryByTestId("conv-turnrail")).toBeNull();
    cleanup();
    server = new FakeClient();
    serve(3);
    mount({ features: { turnRail: false } });
    await settle();
    expect(screen.queryByTestId("conv-turnrail")).toBeNull();
    expect(server.called("harnix.session.outline")).toHaveLength(0);
  });

  it("lists the turn that is running before the outline has it, and reads the outline again when it settles", async () => {
    serve(2);
    mount();
    await settle();
    expect(ticks()).toHaveLength(2);
    act(() => {
      server.push("harnix.event", { session_id: "S1", seq: 1, turn_id: "live", event: { kind: "user_message", text: "and now?" } });
    });
    expect(ticks().map((t) => t.dataset.key)).toEqual(["t0", "t1", "live"]);
    const reads = server.called("harnix.session.outline").length;
    await act(async () => {
      server.push("harnix.turn_settled", { instance: "main", session_id: "S1", turn_id: "live" });
      await flush();
    });
    expect(server.called("harnix.session.outline").length).toBe(reads + 1);
  });

  it("reads older history in before landing on a turn that is not shown, and mounts only the ticks in view", async () => {
    // 1000 messages are read at open: of 510 turns, the first five are not shown.
    serve(510);
    mount();
    await settle();
    expect(screen.getByTestId("conv-turnrail").dataset.count).toBe("510");
    expect(ticks().length).toBeLessThan(120);
    expect(document.querySelector('[data-turn-key="t0"]')).toBeNull();
    expect(document.body.textContent).toMatch(/earlier messages|未显示/);

    // Bring the first tick into the rail's view, then choose it.
    const track = screen.getByTestId("conv-turnrail").querySelector<HTMLElement>(".hx-turnrail-track")!;
    track.scrollTop = 0;
    fireEvent.scroll(track);
    await act(async () => {
      fireEvent.click(tick(0));
      await flush();
    });
    const read = server.called("engine.session.history").at(-1)!.params;
    expect([read.offset, read.limit]).toEqual([0, 20]);
    const first = document.querySelector('[data-turn-key="t0"]');
    expect(first?.textContent).toContain("q0");
    // Nothing older is left to say is hidden.
    expect(document.body.textContent).not.toMatch(/earlier messages|未显示/);
  });

  it("goes to the previous turn with ⌥↑, unless the focus is in a field with text in it", async () => {
    const registry = new Registry();
    registerTurnShortcuts(registry, "builtin");
    const command = (id: string) => registry.at("shortcut").find((c) => c.id === id) as ShortcutCommand;
    expect(registry.at("shortcut").map((c) => [c.id, c.keys])).toEqual([
      ["chat.turn.previous", "Alt+ArrowUp"],
      ["chat.turn.next", "Alt+ArrowDown"],
    ]);
    // No rail on the page: the keys are left to it.
    expect(command("chat.turn.previous").enabled!()).toBe(false);

    serve(3);
    mount();
    await settle();
    const previous = command("chat.turn.previous");
    expect(previous.enabled!()).toBe(true);
    act(() => void previous.run({ space: "main", session: null, scope: "global" }));
    expect(tick(1).getAttribute("aria-current")).toBe("true");

    const input = screen.getByTestId("conv-input") as HTMLTextAreaElement;
    act(() => input.focus());
    expect(previous.enabled!()).toBe(true);
    fireEvent.change(input, { target: { value: "draft" } });
    expect(previous.enabled!()).toBe(false);
  });
});
