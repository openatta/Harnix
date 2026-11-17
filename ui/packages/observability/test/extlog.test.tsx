/**
 * The extension log, a block of the settings' diagnostics: read through
 * `observability.extensions.log`, filtered by extension and least level, followed while live from
 * the last line it has, and copied as text.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { Registry, RegistryProvider } from "@harnix/react";
import type { ExtensionLog, ExtensionLogLine } from "@harnix/core";

import { EXTLOG_EVERY } from "../src/extlog";
import { EXTLOG, ExtensionLogView, lineText, merged, register } from "../src/index";
import { scripted } from "./host";

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

/** A host answering `observability.extensions.log` with `answer`, its notices kept. */
function logHost(answer: (params: unknown) => unknown) {
  const s = scripted((method, params) => (method === "observability.extensions.log" ? answer(params) : {}));
  const notices: string[] = [];
  Object.assign(s.host, { shell: { notice: (text: string) => void notices.push(text) } });
  const asked = () => s.calls.filter((c) => c.method === "observability.extensions.log").map((c) => c.params as { after: number; extension: unknown; level: unknown });
  return { s, notices, asked };
}

function view(host: ReturnType<typeof logHost>) {
  return render(<RegistryProvider registry={new Registry()}><ExtensionLogView host={host.s.host} /></RegistryProvider>);
}

describe("the extension log", () => {
  it("is a block of the settings' diagnostics, the observability's", () => {
    const { s } = logHost(() => log([]));
    const r = new Registry();
    r.activate("observability", () => register(r, s.host));
    r.settle();
    const block = r.at("settings.section").find((c) => c.id === EXTLOG);
    expect([block?.owner, block?.within, block?.label()]).toEqual(["observability", "diagnostics", "Extension log"]);
  });

  it("reads the log, and asks again for what came after the last line it has", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let round = 0;
    const host = logHost((params) => {
      round += 1;
      const after = (params as { after: number }).after;
      return round === 1 ? log([line(1, "flow"), line(2, "acme.wasm", "warn")]) : log([line(1, "flow"), line(2, "acme.wasm"), line(3, "flow")].filter((l) => l.seq > after));
    });
    view(host);
    await act(tick);
    expect(screen.getAllByTestId("extlog-line").map((el) => el.dataset.extension)).toEqual(["flow", "acme.wasm"]);
    expect(screen.getAllByTestId("extlog-line")[1]!.dataset.level).toBe("warn");
    await act(async () => {
      vi.advanceTimersByTime(EXTLOG_EVERY);
      await tick();
    });
    expect(host.asked()[0]).toMatchObject({ after: 0, extension: null, level: null });
    expect(host.asked().at(-1)!.after).toBe(2);
    expect(screen.getAllByTestId("extlog-line").map((el) => el.textContent)).toEqual([
      expect.stringContaining("line 1"),
      expect.stringContaining("line 2"),
      expect.stringContaining("line 3"),
    ]);
  });

  it("starts over when the filter changes, and stops asking when it is not live", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const host = logHost((params) => {
      const p = params as { extension: string | null; level: string | null };
      const all = [line(1, "flow", "error"), line(2, "acme.wasm", "info")];
      return log(all.filter((l) => (!p.extension || l.extension === p.extension) && (p.level !== "error" || l.level === "error")), 2);
    });
    view(host);
    await act(tick);
    await act(async () => {
      fireEvent.change(screen.getByTestId("extlog-extension"), { target: { value: "acme.wasm" } });
      await tick();
    });
    const last = host.asked().at(-1)!;
    expect([last.extension, last.after]).toEqual(["acme.wasm", 0]);
    expect(screen.getAllByTestId("extlog-line").map((el) => el.dataset.extension)).toEqual(["acme.wasm"]);
    await act(async () => {
      fireEvent.change(screen.getByTestId("extlog-extension"), { target: { value: "" } });
      fireEvent.change(screen.getByTestId("extlog-level"), { target: { value: "error" } });
      await tick();
    });
    expect(host.asked().at(-1)!.level).toBe("error");
    expect(screen.getAllByTestId("extlog-line").map((el) => el.dataset.level)).toEqual(["error"]);
    fireEvent.click(screen.getByTestId("extlog-live"));
    expect(screen.getByTestId("extlog-live").getAttribute("aria-pressed")).toBe("false");
    const before = host.asked().length;
    await act(async () => {
      vi.advanceTimersByTime(EXTLOG_EVERY * 3);
      await tick();
    });
    expect(host.asked().length).toBe(before);
  });

  it("copies the lines shown as text", async () => {
    const host = logHost(() => log([line(1, "flow", "warn", "careful")]));
    const written: string[] = [];
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async (text: string) => void written.push(text) } });
    view(host);
    await act(tick);
    await act(async () => {
      fireEvent.click(screen.getByTestId("extlog-copy"));
      await tick();
    });
    expect(written).toEqual(["2026-11-16T09:00:01.000Z WARN flow [call] careful"]);
    expect(host.notices).toEqual(["Lines copied: 1"]);
  });

  it("says when nothing was logged, and what failed when the log cannot be read", async () => {
    view(logHost(() => log([])));
    await act(tick);
    expect(screen.getByTestId("extlog-empty")).toBeTruthy();
    cleanup();
    const refused = Object.assign(new Error("this needs a capability this extension was not granted"), { code: -31000 });
    view(logHost(() => refused));
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
