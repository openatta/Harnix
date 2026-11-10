/** A space with no column of its own lists its sessions in the left column: the shell's list of the front space. */

import { describe, expect, it } from "vitest";
import { act, render } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider } from "@harnix/react";

import { Shell } from "../src/Shell";
import { scriptedApp, tick } from "./scripted";

describe("a space's sessions in the left column", () => {
  it("are read once, not again each time another of them is opened; again when the shell's own list is", async () => {
    const { app, client } = scriptedApp();
    // The scripted client's `notify` is its listeners: a session left would be detached through it.
    const listeners = (client as unknown as { notify: Set<unknown> }).notify;
    Object.assign(client, { notify: Object.assign(() => undefined, { add: (f: unknown) => listeners.add(f), delete: (f: unknown) => listeners.delete(f) }) });
    client.answer = (method) =>
      method === "harnix.sessions.list"
        ? { sessions: [{ session_id: "s1", instance: "main", name: "one" }, { session_id: "s2", instance: "main", name: "two" }] }
        : {};
    render(
      <HarnixUiProvider client={app.conn as HarnixClient}>
        <Shell app={app} />
      </HarnixUiProvider>,
    );
    await act(async () => {
      await app.switchSpace("b");
      await tick();
    });
    const reads = () => client.calls.filter((c) => c.method === "harnix.sessions.list" && c.params.space === "b").length;
    const first = reads();
    expect(first).toBeGreaterThan(0);
    await act(async () => {
      app.open({ instance: "main", sessionId: "s1" });
      await tick();
      app.open({ instance: "main", sessionId: "s2" });
      await tick();
    });
    expect(reads()).toBe(first);
    await act(async () => {
      await app.refreshSessions();
      await tick();
    });
    expect(reads()).toBe(first + 1);
  });
});
