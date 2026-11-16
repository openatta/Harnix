/**
 * The system space's 用量 tab: registered where the console is, nowhere else; the sessions active
 * in the window asked about in batches, by instance, with this device's offset; the figures summed
 * by project, model and day, a bar a day; an instance that cannot be read named; the sections other
 * packages add drawn below, one that throws losing only itself.
 */

import type { ReactNode } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { localOffsetMinutes, windowDays } from "@harnix/core";
import { Registry, RegistryProvider } from "@harnix/react";

import { USAGE_SECTION, register } from "../src/index";
import { SUMMARY_DAYS, UsageSummaryTab, readSummary } from "../src/summary";
import { scripted } from "./host";

afterEach(cleanup);

const tick = () => new Promise((r) => setTimeout(r, 0));
const offset = localOffsetMinutes();
const today = windowDays(Date.now(), 1, offset)[0]!;

const day = (model: string, over: Record<string, number> = {}) => ({
  day: today,
  model,
  calls: 1,
  auxiliary: 0,
  input_tokens: 100,
  cache_read_tokens: 50,
  cache_write_tokens: 10,
  output_tokens: 40,
  model_ms: 1200,
  errors: 0,
  ...over,
});

/** A host whose topics answer at once with these values. */
function hosted(topics: Record<string, unknown>, answer: (method: string, params: unknown) => unknown) {
  const s = scripted(answer);
  s.host.subscribe = (topic: string, params: Record<string, unknown> | undefined, fn: (value: unknown) => void) => {
    const key = params?.instance ? `${topic}:${String(params.instance)}` : topic;
    if (key in topics) fn(topics[key]);
    return () => undefined;
  };
  return s;
}

const NOW = new Date().toISOString();
const TOPICS = {
  "harnix.instances": { instances: [{ id: "main" }, { id: "side" }] },
  "harnix.instances.sessions:main": {
    sessions: [
      { session_id: "a", project_root: "/work/alpha", last_active: NOW },
      { session_id: "b", project_root: null, last_active: NOW },
      { session_id: "old", project_root: "/work/alpha", last_active: "2020-01-01T00:00:00Z" },
    ],
  },
  "harnix.instances.sessions:side": { sessions: [{ session_id: "c", last_active: NOW }] },
};

function answering(method: string, params: unknown): unknown {
  const p = params as { instance: string; session_ids: string[] };
  if (method !== "observability.usage.daily") return new Error(`no ${method}`);
  if (p.instance === "side") return new Error("the instance is down");
  return {
    instance: "main",
    days: SUMMARY_DAYS,
    sessions: {
      a: { recording: true, run: null, days: [day("big"), day("small", { calls: 2, auxiliary: 1 })] },
      b: { recording: true, run: null, stale: true, days: [day("big", { errors: 1 })] },
    },
  };
}

describe("where the tab is", () => {
  it("a tab of the console where the system space defines it, and the point for sections either way", () => {
    const alone = new Registry();
    alone.activate("observability", () => register(alone, scripted(() => ({})).host));
    alone.settle();
    expect(alone.list("system.console.tab")).toEqual([]);
    expect(alone.definitions().map((d) => d.name)).toContain(USAGE_SECTION);

    const withSystem = new Registry();
    withSystem.activate("system", () => withSystem.definePoint("system.console.tab", { kind: "register" }));
    withSystem.activate("observability", () => register(withSystem, scripted(() => ({})).host));
    withSystem.settle();
    expect(withSystem.list<{ id: string; order: number }>("system.console.tab").map((t) => [t.id, t.order])).toEqual([["observability.summary", 40]]);
  });
});

describe("reading the summary", () => {
  it("asks about the sessions active in the window, by instance, with the offset; names an instance it cannot read", async () => {
    const s = hosted(TOPICS, answering);
    const summary = await readSummary(s.host, offset);
    const asked = s.calls.map((c) => c.params as { instance: string; session_ids: string[]; days: number; tz_offset_min: number });
    expect(asked.find((p) => p.instance === "main")).toMatchObject({ session_ids: ["a", "b"], days: SUMMARY_DAYS, tz_offset_min: offset });
    expect(summary.unread).toEqual(["side"]);
    expect(summary.stale).toBe(1);
    expect(summary.days).toHaveLength(SUMMARY_DAYS);
    expect(summary.days[SUMMARY_DAYS - 1]).toBe(today);
    expect(summary.breakdown.byProject.map((r) => [r.key, r.sessions, r.calls])).toEqual([
      ["/work/alpha", 1, 3],
      ["", 1, 1],
    ]);
  });

  it("asks at most two hundred sessions at a time", async () => {
    const many = Array.from({ length: 450 }, (_, i) => ({ session_id: `s${i}`, last_active: NOW }));
    const s = hosted({ "harnix.instances": { instances: [{ id: "main" }] }, "harnix.instances.sessions:main": { sessions: many } }, () => ({ sessions: {} }));
    const summary = await readSummary(s.host, offset);
    expect(s.calls.map((c) => (c.params as { session_ids: string[] }).session_ids.length)).toEqual([200, 200, 50]);
    expect(summary.breakdown.totals.sessions).toBe(0);
  });
});

describe("the tab", () => {
  function shown(topics: Record<string, unknown>, sections: { id: string; render: (c: { since: number; days: number }) => unknown }[] = []) {
    const s = hosted(topics, answering);
    const registry = new Registry();
    registry.activate("system", () => registry.definePoint("system.console.tab", { kind: "register" }));
    registry.activate("observability", () => register(registry, s.host));
    for (const section of sections) registry.activate("flow", () => registry.register(USAGE_SECTION, { ...section, title: () => section.id }, "flow"));
    registry.settle();
    const tab = registry.list<{ render: () => ReactNode }>("system.console.tab")[0]!;
    const view = render(<RegistryProvider registry={registry}>{tab.render()}</RegistryProvider>);
    return { s, view };
  }

  it("totals, a bar a day, and tables by project, model and day that sort by a header", async () => {
    const { view } = shown(TOPICS);
    await act(tick);
    await act(tick);
    const totals = view.getByTestId("usage-summary-totals");
    expect(totals.querySelector('[data-cell="sessions"] .v')?.textContent).toBe("2");
    expect(totals.querySelector('[data-cell="calls"] .v')?.textContent).toBe("4 +1");
    expect(view.getByTestId("usage-summary-unread").textContent).toContain("side");
    expect(view.getByTestId("usage-summary-bars").querySelectorAll(".col")).toHaveLength(SUMMARY_DAYS);
    const projects = view.getByTestId("usage-by-project");
    expect([...projects.querySelectorAll("tbody tr")].map((tr) => tr.getAttribute("data-key"))).toEqual(["/work/alpha", ""]);
    expect(projects.querySelector('tbody tr td[data-column="key"]')?.textContent).toBe("alpha");
    const models = view.getByTestId("usage-by-model");
    expect([...models.querySelectorAll("tbody tr")].map((tr) => tr.getAttribute("data-key"))).toEqual(["big", "small"]);
    fireEvent.click(models.querySelector('th[data-column="key"] button')!);
    expect([...models.querySelectorAll("tbody tr")].map((tr) => tr.getAttribute("data-key"))).toEqual(["big", "small"]);
    fireEvent.click(models.querySelector('th[data-column="key"] button')!);
    expect([...models.querySelectorAll("tbody tr")].map((tr) => tr.getAttribute("data-key"))).toEqual(["small", "big"]);
    expect(models.querySelector('th[data-column="key"]')?.getAttribute("aria-sort")).toBe("descending");
    expect(view.getByTestId("usage-by-day").querySelectorAll("tbody tr")).toHaveLength(1);
  });

  it("with nothing recorded, says so; a section added draws with the window, one that throws loses only itself", async () => {
    const seen: number[] = [];
    const { view } = shown({ "harnix.instances": { instances: [] } }, [
      { id: "ok", render: (c) => (seen.push(c.since), <div data-testid="section-ok">{c.days}</div>) },
      {
        id: "bad",
        render: () => {
          throw new Error("no");
        },
      },
    ]);
    await act(tick);
    await act(tick);
    expect(view.getByTestId("usage-summary-none")).toBeTruthy();
    expect(view.getByTestId("section-ok").textContent).toBe(String(SUMMARY_DAYS));
    expect(seen[0]).toBe(Date.parse(`${windowDays(Date.now() - (SUMMARY_DAYS - 1) * 86_400_000, 1, offset)[0]}T00:00:00Z`) - offset * 60_000);
    expect(view.container.querySelector('[data-section="bad"]')).toBeNull();
  });
});
