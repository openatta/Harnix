/**
 * The console's extension log: a tab of the system space's own, read through
 * `system.extensions.log`, filtered by extension and least level, followed while live from the
 * last line it has, and copied as text.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { Registry, RegistryProvider } from "@harnix/react";
import type { ExtensionLog, ExtensionLogLine } from "@harnix/core";

import { CONSOLE_TAB, EXTLOG_EVERY } from "../src/constants";
import { ExtensionLogTab, lineText, merged } from "../src/extlog";
import { register } from "../src/index";
import { fakeHost } from "./fake-host";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const tick = () => new Promise((r) => setTimeout(r, 0));

const line = (seq: number, extension: string, level = "info", message = `line ${seq}`): ExtensionLogLine => ({
  seq,
  at_ms: Date.UTC(2026, 10, 16, 9, 0, seq),
  extension,
  level,
  origin: "call",
  target: "t",
  message,
});

function log(lines: ExtensionLogLine[], last = lines.length ? lines[lines.length - 1]!.seq : 0): ExtensionLog {
  const counts = new Map<string, number>();
  for (const l of lines) counts.set(l.extension, (counts.get(l.extension) ?? 0) + 1);
  return { lines, last, extensions: [...counts].map(([extension, n]) => ({ extension, lines: n })), dropped: 0 };
}

describe("the extension log tab", () => {
  it("is the system space's own console tab", () => {
    const fake = fakeHost();
    const r = new Registry();
    r.activate("system", () => register(r, fake.host));
    r.settle();
    const tab = r.list<{ id: string; title(): string; owner: string }>(CONSOLE_TAB).find((c) => c.id === "system.extlog");
    expect(tab?.owner).toBe("system");
  });

  it("reads the log, and asks again for what came after the last line it has", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const fake = fakeHost();
    let round = 0;
    fake.answer = (method, params) => {
      if (method !== "system.extensions.log") return {};
      round += 1;
      const after = (params as { after: number }).after;
      return round === 1 ? log([line(1, "flow"), line(2, "acme.wasm", "warn")]) : log([line(1, "flow"), line(2, "acme.wasm"), line(3, "flow")].filter((l) => l.seq > after));
    };
    render(<RegistryProvider registry={new Registry()}><ExtensionLogTab host={fake.host} /></RegistryProvider>);
    await act(tick);
    expect(screen.getAllByTestId("extlog-line").map((el) => el.dataset.extension)).toEqual(["flow", "acme.wasm"]);
    expect(screen.getAllByTestId("extlog-line")[1]!.dataset.level).toBe("warn");
    await act(async () => {
      vi.advanceTimersByTime(EXTLOG_EVERY);
      await tick();
    });
    const asked = fake.calls.filter((c) => c.method === "system.extensions.log").map((c) => c.params as { after: number; extension: unknown; level: unknown });
    expect(asked[0]).toMatchObject({ after: 0, extension: null, level: null });
    expect(asked.at(-1)!.after).toBe(2);
    expect(screen.getAllByTestId("extlog-line").map((el) => el.textContent)).toEqual([
      expect.stringContaining("line 1"),
      expect.stringContaining("line 2"),
      expect.stringContaining("line 3"),
    ]);
  });

  it("starts over when the filter changes, and stops asking when it is not live", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const fake = fakeHost();
    fake.answer = (method, params) => {
      if (method !== "system.extensions.log") return {};
      const p = params as { extension: string | null; level: string | null };
      const all = [line(1, "flow", "error"), line(2, "acme.wasm", "info")];
      return log(all.filter((l) => (!p.extension || l.extension === p.extension) && (p.level !== "error" || l.level === "error")), 2);
    };
    render(<RegistryProvider registry={new Registry()}><ExtensionLogTab host={fake.host} /></RegistryProvider>);
    await act(tick);
    await act(async () => {
      fireEvent.change(screen.getByTestId("extlog-extension"), { target: { value: "acme.wasm" } });
      await tick();
    });
    const last = fake.calls.at(-1)!.params as { extension: string; after: number };
    expect([last.extension, last.after]).toEqual(["acme.wasm", 0]);
    expect(screen.getAllByTestId("extlog-line").map((el) => el.dataset.extension)).toEqual(["acme.wasm"]);
    await act(async () => {
      fireEvent.change(screen.getByTestId("extlog-extension"), { target: { value: "" } });
      fireEvent.change(screen.getByTestId("extlog-level"), { target: { value: "error" } });
      await tick();
    });
    expect((fake.calls.at(-1)!.params as { level: string }).level).toBe("error");
    expect(screen.getAllByTestId("extlog-line").map((el) => el.dataset.level)).toEqual(["error"]);
    fireEvent.click(screen.getByTestId("extlog-live"));
    expect(screen.getByTestId("extlog-live").getAttribute("aria-pressed")).toBe("false");
    const before = fake.calls.length;
    await act(async () => {
      vi.advanceTimersByTime(EXTLOG_EVERY * 3);
      await tick();
    });
    expect(fake.calls.length).toBe(before);
  });

  it("copies the lines shown as text", async () => {
    const fake = fakeHost();
    fake.answer = () => log([line(1, "flow", "warn", "careful")]);
    const written: string[] = [];
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async (text: string) => void written.push(text) } });
    render(<RegistryProvider registry={new Registry()}><ExtensionLogTab host={fake.host} /></RegistryProvider>);
    await act(tick);
    await act(async () => {
      fireEvent.click(screen.getByTestId("extlog-copy"));
      await tick();
    });
    expect(written).toEqual(["2026-11-16T09:00:01.000Z WARN flow [call] careful"]);
    expect(fake.shown.notices).toEqual(["Lines copied: 1"]);
  });

  it("says when nothing was logged, and what failed when the log cannot be read", async () => {
    const fake = fakeHost();
    fake.answer = () => log([]);
    render(<RegistryProvider registry={new Registry()}><ExtensionLogTab host={fake.host} /></RegistryProvider>);
    await act(tick);
    expect(screen.getByTestId("extlog-empty")).toBeTruthy();
    cleanup();
    const broken = fakeHost();
    broken.answer = () => {
      throw { code: -31000, message: "this needs a capability this extension was not granted" };
    };
    render(<RegistryProvider registry={new Registry()}><ExtensionLogTab host={broken.host} /></RegistryProvider>);
    await act(tick);
    expect(screen.getByTestId("extlog-failed").dataset.code).toBe("-31000");
  });
});

describe("the log's lines", () => {
  it("are added once when two reads overlap, and no more than the page keeps", () => {
    const now = [line(1, "a"), line(2, "a")];
    expect(merged(now, [line(2, "a"), line(3, "a")]).map((l) => l.seq)).toEqual([1, 2, 3]);
    expect(merged(now, [line(1, "a")])).toBe(now);
    const many = Array.from({ length: 1200 }, (_, i) => line(i + 1, "a"));
    expect(merged([], many).length).toBe(1000);
  });

  it("are copied with their time, level, extension and origin", () => {
    expect(lineText(line(5, "flow", "error", "broke"))).toBe("2026-11-16T09:00:05.000Z ERROR flow [call] broke");
  });
});
