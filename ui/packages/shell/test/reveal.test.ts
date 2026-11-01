/** `session.open(ref, {reveal})` and `session.reveal(key)` (ipdrun-40): find an entry once caught up. */

import { afterEach, describe, expect, it, vi } from "vitest";
import type { SessionController } from "@harnix/core/state";

import { hostFor } from "../src/host";
import { REVEAL_MS } from "../src/reveal";
import { scriptedApp } from "./scripted";

afterEach(() => {
  document.body.innerHTML = "";
  vi.useRealTimers();
});

/** What the conversation of `sessionId` would have drawn, and a controller for it. */
function conversation(sessionId: string, inner: string) {
  document.body.innerHTML = `<section id="main"><section data-testid="conv" data-id="${sessionId}">${inner}</section></section>`;
  let loading = true;
  const controller = {
    instance: "system",
    sessionId,
    getSnapshot: () => ({ sessionId, loading, failed: null }),
  } as unknown as SessionController;
  return { controller, caughtUp: () => (loading = false) };
}

describe("reveal", () => {
  it("open with reveal waits for the transcript, then scrolls to the entry and marks it", async () => {
    const { app } = scriptedApp();
    const host = hostFor(app);
    const ref = { instance: "system", sessionId: "s1" };
    const view = conversation("s1", `<div class="hx-blk" data-id="b1"></div><div data-reveal="P-7"><span>card</span></div>`);
    const target = document.querySelector('[data-reveal="P-7"]') as HTMLElement;
    const scrolled = vi.fn();
    target.scrollIntoView = scrolled;

    const found = host.session.open(ref, { reveal: "P-7" });
    expect(found).toBeInstanceOf(Promise);
    // The conversation is drawn and catching up: nothing is marked yet.
    app.controller = view.controller;
    await new Promise((r) => setTimeout(r, 120));
    expect(target.hasAttribute("data-revealed")).toBe(false);
    view.caughtUp();
    expect(await found).toBe(true);
    expect(target.hasAttribute("data-revealed")).toBe(true);
    expect(scrolled).toHaveBeenCalledWith({ block: "center", behavior: "smooth" });

    // A block id is an entry too; the mark goes after a while.
    vi.useFakeTimers();
    expect(await host.session.reveal("b1")).toBe(true);
    const block = document.querySelector('.hx-blk[data-id="b1"]')!;
    expect(block.hasAttribute("data-revealed")).toBe(true);
    vi.advanceTimersByTime(REVEAL_MS + 1);
    expect(block.hasAttribute("data-revealed")).toBe(false);
  });

  it("is false for an entry that is not there, with no session open, or once another session is open", async () => {
    const { app } = scriptedApp();
    const host = hostFor(app);
    expect(await host.session.reveal("x")).toBe(false);

    const view = conversation("s1", "");
    view.caughtUp();
    app.controller = view.controller;
    app.open({ instance: "system", sessionId: "s1" });
    const started = Date.now();
    expect(await host.session.reveal("missing")).toBe(false);
    expect(Date.now() - started).toBeLessThan(4000);

    const pending = host.session.open({ instance: "system", sessionId: "s2" }, { reveal: "x" });
    app.open({ instance: "system", sessionId: "s3" });
    expect(await pending).toBe(false);
  });

  it("without reveal, open is as before", () => {
    const { app } = scriptedApp();
    expect(hostFor(app).session.open({ instance: "system", sessionId: "s1" })).toBeUndefined();
    expect(app.snapshot.current).toEqual({ instance: "system", sessionId: "s1" });
  });
});
