/**
 * The home page's blocks from this package: registered where the Harnix app defines the point,
 * nowhere else; the process at a glance — the versions, the instances, the main Agents active and
 * all, the sessions running a turn, the background tasks — and what waits, from the count the page
 * hands its blocks, opening the list.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { Registry } from "@harnix/react";

import { GlobalStats, register } from "../src/index";
import { scripted } from "./host";

afterEach(cleanup);

const HOME_PANELS = "harnix-app.home.panels";

const TOPICS: Record<string, unknown> = {
  "observability.process": { harnix: "0.4.5-2026111601", agix: "0.3.16-2026111505", uptime_secs: 65, peak_rss_bytes: 80 * 1048576, threads: 12, connections: [{ client: "c1", device: "local", topology: "single_duplex", user_agent: "Mozilla/5.0" }] },
  "harnix.instances": { instances: [{ id: "main" }, { id: "side" }] },
  "harnix.instances.sessions:main": {
    sessions: [
      { session_id: "a", running: true, tasks: 2, status: "active" },
      { session_id: "b", waiting: "approval", tasks: 0 },
      { session_id: "c", tasks: 0 },
    ],
  },
  "harnix.instances.sessions:side": { sessions: [{ session_id: "d", tasks: 1 }] },
  "harnix.sessions.status": { sessions: [{ instance: "main", session_id: "a" }] },
};

/** A host whose topics answer at once with these values. */
function hosted() {
  const s = scripted(() => ({}));
  s.host.subscribe = (topic: string, params: Record<string, unknown> | undefined, fn: (value: unknown) => void) => {
    const key = params?.instance ? `${topic}:${String(params.instance)}` : topic;
    if (key in TOPICS) queueMicrotask(() => fn(TOPICS[key]));
    return () => undefined;
  };
  return s;
}

describe("the home page's blocks", () => {
  it("are there only where the Harnix app defines the point: the figures first and wide, the usage after the lists", () => {
    const alone = new Registry();
    alone.activate("observability", () => register(alone, scripted(() => ({})).host));
    alone.settle();
    expect(alone.list(HOME_PANELS)).toEqual([]);

    const r = new Registry();
    r.activate("harnix-app", () => r.definePoint(HOME_PANELS, { kind: "render" }));
    r.activate("observability", () => register(r, scripted(() => ({})).host));
    r.settle();
    const blocks = r.list<{ id: string; order: number; wide?: boolean }>(HOME_PANELS).map((b) => [b.id, b.order, b.wide]);
    expect(blocks).toEqual([
      ["observability.stats", 10, true],
      ["observability.summary", 60, true],
    ]);
  });

  it("the process at a glance: versions, instances, Agents active and all, running sessions and tasks, and what waits opening the list", async () => {
    const s = hosted();
    let opened = 0;
    const page = render(<GlobalStats host={s.host} context={{ waiting: 3, openWaiting: () => (opened += 1) }} />);
    await act(async () => {});
    await act(async () => {});
    const value = (id: string) => page.container.querySelector(`[data-kpi="${id}"] .v`)?.textContent;
    expect([value("harnix"), value("agix"), value("memory"), value("threads"), value("users")]).toEqual(["0.4.5", "0.3.16", "80 MB", "12", "1"]);
    expect([value("instances"), value("agents"), value("sessions"), value("tasks"), value("waiting")]).toEqual(["2", "3/4", "1", "3", "3"]);
    fireEvent.click(page.getByTestId("home-stats-waiting"));
    expect(opened).toBe(1);
  });
});
