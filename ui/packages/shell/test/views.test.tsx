/**
 * The centre's views (`conversation.view`): the switch is drawn only while a session is open and a
 * view applies, the conversation is the default, each session's choice is remembered in this
 * browser and forgotten with the session, and the host API says and sets which view is in front.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { CONVERSATION_VIEW, HarnixUiProvider, Registry } from "@harnix/react";

import { App } from "../src/app";
import { hostFor } from "../src/host";
import { Shell } from "../src/Shell";
import { registerShellShortcuts } from "../src/shortcuts";
import { ScriptedClient, scriptedApp, space, tick } from "./scripted";

afterEach(cleanup);

const s1 = { instance: "main", sessionId: "s1" };
const s2 = { instance: "main", sessionId: "s2" };

function trajectory(app: App, id = "obs.trajectory", extra: Record<string, unknown> = {}) {
  app.registry.register("conversation.view", { id, title: () => "Trajectory", render: (ref) => <div data-testid="traj">{ref.sessionId}</div>, ...extra }, "obs");
}

function draw(app: App) {
  registerShellShortcuts(app);
  return render(
    <HarnixUiProvider client={app.conn as HarnixClient}>
      <Shell app={app} />
    </HarnixUiProvider>,
  );
}

describe("which view a session is shown in", () => {
  it("is the conversation unless chosen, remembered per session in this browser, and forgotten with the session", () => {
    const { app } = scriptedApp();
    trajectory(app);
    app.set({ currentBySpace: { sessions: s1 } });
    expect(app.conversationView(s1)).toBe(CONVERSATION_VIEW);
    const seen: string[] = [];
    app.onView((view) => seen.push(view));

    app.setConversationView(s1, "obs.trajectory");
    expect(app.conversationView(s1)).toBe("obs.trajectory");
    expect(app.conversationView(s2)).toBe(CONVERSATION_VIEW);
    expect(JSON.parse(localStorage.getItem("harnix.view")!)).toEqual({ "main/s1": "obs.trajectory" });
    expect(seen).toEqual(["obs.trajectory"]);

    // Another session in front: the view in front is that session's.
    app.set({ currentBySpace: { sessions: s2 } });
    expect(seen).toEqual(["obs.trajectory", CONVERSATION_VIEW]);
    app.set({ currentBySpace: { sessions: s1 } });
    expect(seen).toEqual(["obs.trajectory", CONVERSATION_VIEW, "obs.trajectory"]);

    // A new page over the same storage remembers; a choice no registration backs is the conversation.
    const again = new App(new ScriptedClient() as unknown as HarnixClient, new Registry());
    again.registry.register("space", space("sessions"), "test");
    expect(again.conversationView(s1)).toBe(CONVERSATION_VIEW);
    trajectory(again);
    expect(again.conversationView(s1)).toBe("obs.trajectory");

    // Back to the conversation: the entry goes; the session deleted: so does its entry.
    app.setConversationView(s1, CONVERSATION_VIEW);
    expect(JSON.parse(localStorage.getItem("harnix.view")!)).toEqual({});
    app.setConversationView(s1, "obs.trajectory");
    app.forget(s1);
    expect(JSON.parse(localStorage.getItem("harnix.view")!)).toEqual({});
  });

  it("lists the views that apply, in order, a match that throws dropping its own", () => {
    const { app } = scriptedApp();
    trajectory(app, "b.late", { order: 50 });
    trajectory(app, "a.early", { order: 10 });
    trajectory(app, "c.other", { match: (ref: { sessionId: string }) => ref.sessionId === "s2" });
    trajectory(app, "d.broken", { match: () => { throw new Error("no"); } });
    expect(app.viewsFor(s1).map((v) => v.id)).toEqual(["a.early", "b.late"]);
    expect(app.viewsFor(s2).map((v) => v.id)).toEqual(["a.early", "b.late", "c.other"]);
    expect(app.viewsFor(null)).toEqual([]);
    // Chosen, then no longer applicable: the conversation, with the choice kept for when it is again.
    app.setConversationView(s1, "c.other");
    expect(app.conversationView(s1)).toBe(CONVERSATION_VIEW);
    app.setConversationView(s2, "c.other");
    expect(app.conversationView(s2)).toBe("c.other");
  });

  it("the host says which view is in front, shows one, tells of changes, and opens a session in one", async () => {
    const { app } = scriptedApp();
    trajectory(app);
    const host = hostFor(app);
    expect(host.conversation.current()).toBe(CONVERSATION_VIEW);
    const seen: string[] = [];
    host.conversation.onChange((view) => seen.push(view));
    // No session open: nothing to show it in.
    host.conversation.show("obs.trajectory");
    expect(host.conversation.current()).toBe(CONVERSATION_VIEW);
    app.set({ currentBySpace: { sessions: s1 } });
    host.conversation.show("obs.trajectory");
    expect(host.conversation.current()).toBe("obs.trajectory");
    expect(seen).toEqual(["obs.trajectory"]);
    await host.session.open(s2, { view: "obs.trajectory" });
    expect(app.snapshot.current).toEqual(s2);
    expect(host.conversation.current()).toBe("obs.trajectory");
  });
});

describe("the switch at the top of the centre", () => {
  it("is drawn only with a session open and a view registered; a click shows the view over the conversation, which stays on the page", async () => {
    const { app } = scriptedApp();
    const { container, unmount } = draw(app);
    expect(container.querySelector('[data-testid="view-switch"]')).toBeNull();
    act(() => app.set({ currentBySpace: { sessions: s1 } }));
    expect(container.querySelector('[data-testid="view-switch"]')).toBeNull();
    act(() => trajectory(app));
    const strip = container.querySelector('[data-testid="view-switch"]')!;
    expect(strip).not.toBeNull();
    expect([...strip.querySelectorAll("[role=tab]")].map((b) => b.getAttribute("data-view"))).toEqual([CONVERSATION_VIEW, "obs.trajectory"]);
    expect(container.querySelector("#main")?.getAttribute("data-view")).toBe(CONVERSATION_VIEW);

    fireEvent.click(strip.querySelector('[data-view="obs.trajectory"]')!);
    await act(tick);
    expect(container.querySelector("#main")?.getAttribute("data-view")).toBe("obs.trajectory");
    expect(container.querySelector('[data-testid="conversation-view"] [data-testid="traj"]')?.textContent).toBe("s1");
    expect(container.querySelector("#main > .hx-chat.under-view")).not.toBeNull();

    fireEvent.click(strip.querySelector(`[data-view="${CONVERSATION_VIEW}"]`)!);
    await act(tick);
    expect(container.querySelector('[data-testid="conversation-view"]')).toBeNull();
    expect(container.querySelector("#main > .hx-chat.under-view")).toBeNull();
    unmount();
  });
});
