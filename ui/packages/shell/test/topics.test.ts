/**
 * Topic subscriptions against a server that handles each frame in its own task: an unsubscribe
 * answered late must not be applied after a subscribe that followed it.
 */

import { describe, expect, it } from "vitest";
import type { HarnixClient } from "@harnix/core";

import { Topics } from "../src/topics";

type Listener = (method: string, params: Record<string, unknown>) => void;

/** A server that applies each frame when it is answered: unsubscribes late, subscribes at once. */
class ReorderingClient {
  status = "open";
  frames: string[] = [];
  /** Whether the server holds the subscription, as of the frames it has applied. */
  serverHolds = false;
  private notify = new Set<Listener>();
  private releases: (() => void)[] = [];

  call(method: string): Promise<unknown> {
    this.frames.push(method);
    if (method === "harnix.subscribe") {
      this.serverHolds = true;
      return Promise.resolve({ value: "now" });
    }
    return new Promise((resolve) => {
      this.releases.push(() => {
        this.serverHolds = false;
        resolve({});
      });
    });
  }
  onNotify(fn: Listener): () => void {
    this.notify.add(fn);
    return () => this.notify.delete(fn);
  }
  onStatus(): () => void {
    return () => undefined;
  }
  /** Apply the unsubscribes that are waiting. */
  release(): void {
    for (const r of this.releases.splice(0)) r();
  }
  push(topic: string, value: unknown): void {
    for (const fn of [...this.notify]) fn("harnix.topic", { topic, params: {}, value });
  }
}

const tick = () => new Promise((r) => setTimeout(r, 0));

describe("Topics resubscribing", () => {
  it("waits for the unsubscribe in flight before subscribing again", async () => {
    const client = new ReorderingClient();
    const topics = new Topics(client as unknown as HarnixClient);
    const first = topics.subscribe("t", {}, () => undefined);
    await tick();
    first();
    const seen: unknown[] = [];
    topics.subscribe("t", {}, (v) => seen.push(v));
    await tick();
    // The unsubscribe has not been answered: no second subscribe may be on the wire yet.
    expect(client.frames).toEqual(["harnix.subscribe", "harnix.unsubscribe"]);
    client.release();
    await tick();
    expect(client.frames).toEqual(["harnix.subscribe", "harnix.unsubscribe", "harnix.subscribe"]);
    expect(client.serverHolds).toBe(true);
    client.push("t", "pushed");
    expect(seen).toEqual(["now", "pushed"]);
  });

  it("subscribes at once when no unsubscribe is in flight", async () => {
    const client = new ReorderingClient();
    const topics = new Topics(client as unknown as HarnixClient);
    topics.subscribe("t", {}, () => undefined)();
    client.release();
    await tick();
    topics.subscribe("t", {}, () => undefined);
    await tick();
    expect(client.frames).toEqual(["harnix.subscribe", "harnix.unsubscribe", "harnix.subscribe"]);
    expect(client.serverHolds).toBe(true);
  });

  it("does not subscribe for a subscriber that left while it waited", async () => {
    const client = new ReorderingClient();
    const topics = new Topics(client as unknown as HarnixClient);
    topics.subscribe("t", {}, () => undefined)();
    await tick();
    topics.subscribe("t", {}, () => undefined)();
    client.release();
    await tick();
    client.release();
    await tick();
    expect(client.serverHolds).toBe(false);
  });
});
