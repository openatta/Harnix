/**
 * The extension log, a block of the settings' diagnostics: what this process logged as each
 * extension's — a native extension while it answered, a WebAssembly package's own log, an extension
 * process's stderr, the host's lines about one — the newest kept (`observability.extensions.log`).
 * Filtered by extension and least level; followed while live; the lines shown copied as text.
 */

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { ExtensionLog, ExtensionLogCount, ExtensionLogLine } from "@harnix/core";
import { useT, type HostApi } from "@harnix/react";

import { Failure } from "./shared.tsx";

/** The diagnostics' block: what `host.settings.open` names to show it. */
export const EXTLOG = "observability.extlog";

/** How often the log is read again while it is live. */
export const EXTLOG_EVERY = 2000;
/** The most lines the page keeps: the process keeps at most 500 an extension. */
export const EXTLOG_KEPT = 1000;

/** The levels a reader may ask for at least; none is every line. */
export const LEVELS = ["error", "warn", "info", "debug"] as const;
type Least = (typeof LEVELS)[number] | "";

/** One line as text: what the copy puts on the clipboard. */
export function lineText(line: ExtensionLogLine): string {
  return `${new Date(line.at_ms).toISOString()} ${line.level.toUpperCase()} ${line.extension} [${line.origin}] ${line.message}`;
}

/** `hh:mm:ss`, local. */
function timeOf(ms: number): string {
  const d = new Date(ms);
  const two = (n: number) => String(n).padStart(2, "0");
  return `${two(d.getHours())}:${two(d.getMinutes())}:${two(d.getSeconds())}`;
}

/** `now` with what came after its last line, at most `EXTLOG_KEPT`: two reads that overlap add a line once. */
export function merged(now: ExtensionLogLine[], more: ExtensionLogLine[]): ExtensionLogLine[] {
  const top = now.length ? now[now.length - 1]!.seq : 0;
  const added = more.filter((l) => l.seq > top);
  return added.length ? [...now, ...added].slice(-EXTLOG_KEPT) : now;
}

export function ExtensionLogView({ host }: { host: HostApi }) {
  const tr = useT();
  const [extension, setExtension] = useState("");
  const [level, setLevel] = useState<Least>("");
  const [live, setLive] = useState(true);
  const [lines, setLines] = useState<ExtensionLogLine[]>([]);
  const [counts, setCounts] = useState<ExtensionLogCount[]>([]);
  const [dropped, setDropped] = useState(0);
  const [loaded, setLoaded] = useState(false);
  const [failure, setFailure] = useState<unknown>(null);
  // What the next read asks after, and which filter it is for: an answer to an older filter is dropped.
  const after = useRef(0);
  const filter = `${extension}\u0000${level}`;
  const current = useRef(filter);
  current.current = filter;
  const read = useRef<() => Promise<void>>(async () => undefined);
  read.current = async () => {
    const asked = filter;
    try {
      const log = await host.call<ExtensionLog>("observability.extensions.log", {
        extension: extension || null,
        level: level || null,
        after: after.current,
        limit: EXTLOG_KEPT,
      });
      if (current.current !== asked) return;
      after.current = Math.max(after.current, log.last);
      setLines((now) => merged(now, log.lines ?? []));
      setCounts(log.extensions ?? []);
      setDropped(log.dropped ?? 0);
      setFailure(null);
      setLoaded(true);
    } catch (e) {
      if (current.current === asked) setFailure(e ?? "error");
    }
  };
  // A new filter starts over.
  useEffect(() => {
    after.current = 0;
    setLines([]);
    setLoaded(false);
    void read.current();
  }, [host, filter]);
  useEffect(() => {
    if (!live) return;
    const timer = setInterval(() => void read.current(), EXTLOG_EVERY);
    return () => clearInterval(timer);
  }, [live, filter]);
  // Live, the newest line stays in view, unless the reader scrolled up to look at an older one.
  const box = useRef<HTMLDivElement>(null);
  const atEnd = useRef(true);
  useLayoutEffect(() => {
    const el = box.current;
    if (el && live && atEnd.current) el.scrollTop = el.scrollHeight;
  }, [lines, live]);
  const copy = () => {
    const text = lines.map(lineText).join("\n");
    void navigator.clipboard
      ?.writeText(text)
      .then(() => host.shell.notice(tr("obs.extlog.copied", { count: lines.length })))
      .catch(() => undefined);
  };
  // The extension asked for stays in the list while it has no lines.
  const known = counts.some((c) => c.extension === extension) || !extension ? counts : [...counts, { extension, lines: 0 }];
  return (
    <div className="obs-extlog" data-testid="extlog">
      <div className="hint">{tr("obs.extlog.about")}</div>
      <div className="tools-row">
        <select value={extension} onChange={(e) => setExtension(e.target.value)} data-testid="extlog-extension" aria-label={tr("obs.extlog.extension")}>
          <option value="">{tr("obs.extlog.all")}</option>
          {known.map((c) => (
            <option key={c.extension} value={c.extension}>
              {tr("obs.extlog.extensionCount", { extension: c.extension, count: c.lines })}
            </option>
          ))}
        </select>
        <select value={level} onChange={(e) => setLevel(e.target.value as Least)} data-testid="extlog-level" aria-label={tr("obs.extlog.level")}>
          <option value="">{tr("obs.extlog.level.all")}</option>
          {LEVELS.map((l) => (
            <option key={l} value={l}>
              {tr("obs.extlog.level.atLeast", { level: l })}
            </option>
          ))}
        </select>
        <button type="button" className={`btn sm outline${live ? " on" : ""}`} aria-pressed={live} data-testid="extlog-live" onClick={() => setLive(!live)}>
          {tr("obs.extlog.live")}
        </button>
        <button type="button" className="btn sm outline" data-testid="extlog-copy" disabled={!lines.length} onClick={copy}>
          {tr("obs.extlog.copy")}
        </button>
        {dropped ? <span className="note end" data-testid="extlog-dropped">{tr("obs.extlog.dropped", { count: dropped })}</span> : null}
      </div>
      {failure ? <Failure error={failure} testId="extlog-failed" /> : null}
      {!failure && !loaded ? <div className="note">{tr("obs.extlog.loading")}</div> : null}
      {!failure && loaded && !lines.length ? <div className="note" data-testid="extlog-empty">{tr("obs.extlog.empty")}</div> : null}
      <div
        className="extlog-lines"
        ref={box}
        role="log"
        aria-live={live ? "polite" : "off"}
        onScroll={(e) => {
          const el = e.currentTarget;
          atEnd.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
        }}
      >
        {lines.map((line) => (
          <div key={line.seq} className="extlog-line" data-testid="extlog-line" data-extension={line.extension} data-level={line.level} data-origin={line.origin}>
            <span className="at" title={new Date(line.at_ms).toISOString()}>{timeOf(line.at_ms)}</span>
            <span className={`lv lv-${line.level}`}>{line.level}</span>
            <span className="ext">{line.extension}</span>
            <span className="origin" title={line.target}>{tr(`obs.extlog.origin.${line.origin}`)}</span>
            <span className="msg">{line.message}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
