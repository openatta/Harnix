/** The calls tab: every model call, what went in and what came out. */

import { useEffect, useRef, useState } from "react";
import { errorText } from "@harnix/core";
import { Disclosure, clip, useT } from "@harnix/react";

import type { App, SessionKey } from "../../app.ts";
import { Glyph } from "../../glyph.tsx";
import { messageText, usageText } from "../format.ts";
import { locateInConversation } from "../navigate.ts";
import type { TraceCall } from "../types.ts";
import type { View } from "../view.ts";

type Filter = "all" | "failed" | "tools";

function failed(call: TraceCall): boolean {
  return call.outcome?.status === "error" || call.tools.some((tool) => tool.is_error);
}

export function CallsTab({ app, view, session }: { app: App; view: View; session: SessionKey }) {
  const tr = useT();
  // A jump from the conversation names a tool call: the call that asked for it is picked out.
  const [focus] = useState(() => view.takeTool());
  const [calls, setCalls] = useState<TraceCall[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const [range, setRange] = useState<[number, number] | null>(null);
  const [open, setOpen] = useState<number | null>(null);
  const list = useRef<HTMLDivElement>(null);
  const scroll = useRef(0);
  useEffect(() => {
    let alive = true;
    const load = () =>
      app
        .call<{ calls?: TraceCall[] }>("harnix.system.trace", { instance: session.instance, session_id: session.sessionId })
        .then((r) => alive && (setCalls(r.calls ?? []), setError(null)))
        .catch((e) => alive && setError(errorText(e)));
    void load();
    const off = app.client.onNotify((method, params) => {
      if (method === "harnix.turn_settled" && params.session_id === session.sessionId) void load();
    });
    return () => {
      alive = false;
      off();
    };
  }, [app, session.instance, session.sessionId]);
  useEffect(() => {
    if (open === null && list.current) list.current.scrollTop = scroll.current;
  }, [open]);
  useEffect(() => {
    if (focus && calls) list.current?.querySelector(`[data-tool="${CSS.escape(focus)}"]`)?.scrollIntoView({ block: "center" });
  }, [focus, calls]);
  if (error) return <div className="empty">{error}</div>;
  if (!calls) return null;
  if (open !== null) {
    const previous = calls.filter((c) => c.index < open && !c.purpose).pop() ?? null;
    return <CallDetail app={app} session={session} call={calls.find((c) => c.index === open)!} previous={previous} onBack={() => setOpen(null)} />;
  }
  if (!calls.length) return <div className="empty">{tr("system.noCalls")}</div>;
  const needle = query.trim().toLowerCase();
  const shown = calls.filter(
    (call) =>
      (filter === "failed" ? failed(call) : filter === "tools" ? call.tools.length > 0 : true) &&
      (!needle || searchable(call).includes(needle)) &&
      (!range || overlaps(call, range)),
  );
  const firstFailure = calls.find(failed);
  const turns = [...new Set(shown.map((c) => c.turn))];
  return (
    <div className="system-pane calls" data-testid="system-calls" ref={list}>
      <div className="tools-row">
        {(["all", "failed", "tools"] as Filter[]).map((f) => (
          <button type="button" key={f} className={`btn sm outline${filter === f ? " on" : ""}`} data-filter={f} onClick={() => setFilter(f)}>
            {tr(`system.filter.${f}`)}
          </button>
        ))}
        {firstFailure ? (
          <button
            type="button"
            className="btn sm outline end"
            data-testid="jump-to-failure"
            onClick={() => document.querySelector(`[data-call="${firstFailure.index}"]`)?.scrollIntoView({ block: "center" })}
          >
            {tr("system.jumpToFailure")}
          </button>
        ) : null}
      </div>
      <div className="tools-row">
        <input
          className="search"
          type="search"
          data-testid="calls-search"
          placeholder={tr("system.searchCalls")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        {range ? (
          <button type="button" className="btn sm outline end" data-testid="timeline-clear" onClick={() => setRange(null)}>
            {tr("system.clearRange")}
          </button>
        ) : null}
      </div>
      <Timeline calls={calls} range={range} onRange={setRange} />
      {!shown.length ? <div className="note" data-testid="calls-none">{tr("system.noMatchingCalls")}</div> : null}
      {turns.map((turn) => (
        <div key={turn}>
          <div className="turnh">{tr("system.turn", { turn })}</div>
          {shown
            .filter((c) => c.turn === turn)
            .map((call) => (
              <div key={call.index}>
                <button
                  type="button"
                  className={`step k-model${failed(call) && call.outcome?.status === "error" ? " bad" : ""}${focus && call.tools.some((tool) => tool.id === focus) ? " on" : ""}`}
                  data-call={call.index}
                  data-testid="system-call"
                  onClick={() => {
                    scroll.current = list.current?.scrollTop ?? 0;
                    setOpen(call.index);
                  }}
                >
                  <div className="l1">
                    <span className="kd2">{call.purpose ? call.purpose : tr("system.step.model")}</span>
                    <b>#{call.index + 1}</b>
                    <span className="r">
                      {call.model} · {usageText(call.usage)} · {call.duration_ms != null ? `${(call.duration_ms / 1000).toFixed(1)}s` : "—"}
                      {call.timing?.ttft_ms != null ? <span data-testid="call-ttft"> · {tr("system.ttft", { ms: call.timing.ttft_ms })}</span> : null}
                      {call.timing?.tokens_per_s != null ? ` · ${tr("system.rate", { rate: call.timing.tokens_per_s })}` : ""} · {call.stop_reason ?? call.outcome?.status}
                    </span>
                  </div>
                  <div className="l2">
                    <span className="in">{tr("system.callIn", { blocks: call.input.system.length, tools: call.input.tools, messages: call.input.messages })}</span>
                    <span className="arrow">→</span>
                    <span className="out">{callOut(call, tr)}</span>
                  </div>
                </button>
                {call.tools.map((tool) => (
                  <div key={tool.id} className={`step k-tool nested${tool.is_error ? " bad" : ""}${tool.id === focus ? " on" : ""}`} data-tool={tool.id}>
                    <div className="l1">
                      <span className="kd2">{tr("system.step.tool")}</span>
                      <b>{tool.name}</b>
                      <span className="r">{tool.result_chars != null ? `${tool.result_chars} chars` : tr("system.noResultYet")}</span>
                    </div>
                    <div className="l2">
                      <span className="in">{tool.input}</span>
                      <span className="arrow">→</span>
                      <span className="out">{tool.result ?? "…"}</span>
                    </div>
                  </div>
                ))}
              </div>
            ))}
        </div>
      ))}
    </div>
  );
}

/** What a search over the calls looks in: the answer, the model, what it was for, its tool calls. */
function searchable(call: TraceCall): string {
  return [call.output.text, call.model, call.purpose ?? "", ...call.tools.flatMap((tool) => [tool.name, tool.input, tool.result ?? ""])].join("\n").toLowerCase();
}

/** A call's stretch of time: from being sent to its end. */
function spanOf(call: TraceCall): [number, number] | null {
  const sent = call.timing?.sent_at;
  if (sent == null) return null;
  return [sent, sent + (call.duration_ms ?? 0)];
}

function overlaps(call: TraceCall, [from, to]: [number, number]): boolean {
  const span = spanOf(call);
  return span != null && span[0] <= to && span[1] >= from;
}

/**
 * The calls on one time line: each call a bar from when it was sent to its end, the wait for its
 * first chunk drawn lighter; between two calls of a turn, the tools ran. Dragging across it picks a
 * stretch of time, and the list below shows only the calls in it.
 */
function Timeline({ calls, range, onRange }: { calls: TraceCall[]; range: [number, number] | null; onRange: (r: [number, number] | null) => void }) {
  const tr = useT();
  const strip = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<[number, number] | null>(null);
  const spans = calls.map((call) => ({ call, span: spanOf(call) })).filter((x): x is { call: TraceCall; span: [number, number] } => x.span != null);
  if (!spans.length) return null;
  const start = Math.min(...spans.map((x) => x.span[0]));
  const end = Math.max(...spans.map((x) => x.span[1]));
  const total = Math.max(end - start, 1);
  const pct = (t: number) => `${((t - start) / total) * 100}%`;
  const width = (from: number, to: number) => `${(Math.max(to - from, 0) / total) * 100}%`;
  const at = (clientX: number) => {
    const box = strip.current!.getBoundingClientRect();
    return start + Math.min(Math.max((clientX - box.left) / box.width, 0), 1) * total;
  };
  const shownRange = drag ?? range;
  const gaps: { key: string; from: number; to: number }[] = [];
  for (let i = 1; i < spans.length; i += 1) {
    const a = spans[i - 1]!;
    const b = spans[i]!;
    if (a.call.turn === b.call.turn && !b.call.purpose && b.span[0] > a.span[1]) gaps.push({ key: `${a.call.index}-${b.call.index}`, from: a.span[1], to: b.span[0] });
  }
  return (
    <div className="timeline-wrap">
      <div
        className="timeline"
        ref={strip}
        data-testid="calls-timeline"
        title={tr("system.timelineHint")}
        onPointerDown={(e) => {
          strip.current?.setPointerCapture?.(e.pointerId);
          const t = at(e.clientX);
          setDrag([t, t]);
        }}
        onPointerMove={(e) => drag && setDrag([drag[0], at(e.clientX)])}
        onPointerUp={() => {
          if (!drag) return;
          const [a, b] = drag[0] <= drag[1] ? drag : [drag[1], drag[0]];
          setDrag(null);
          onRange(b - a >= total / 200 ? [a, b] : null);
        }}
      >
        {gaps.map((gap) => (
          <i key={gap.key} className="gap" style={{ left: pct(gap.from), width: width(gap.from, gap.to) }} />
        ))}
        {spans.map(({ call, span }) => (
          <i key={call.index} className={`call${call.purpose ? " aux" : ""}`} data-testid="timeline-call" data-call={call.index} style={{ left: pct(span[0]), width: width(span[0], span[1]) }}>
            {call.timing?.ttft_ms ? <b style={{ width: `${Math.min(100, (call.timing.ttft_ms / Math.max(span[1] - span[0], 1)) * 100)}%` }} /> : null}
          </i>
        ))}
        {shownRange ? (
          <i
            className="range"
            data-testid="timeline-range"
            style={{ left: pct(Math.min(...shownRange)), width: width(Math.min(...shownRange), Math.max(...shownRange)) }}
          />
        ) : null}
      </div>
      <div className="legend2">
        <span className="model">{tr("system.legend.model")}</span>
        <span className="wait">{tr("system.legend.wait")}</span>
        <span className="gap">{tr("system.legend.toolsTime")}</span>
        <span className="muted">{tr("system.duration", { seconds: (total / 1000).toFixed(1) })}</span>
      </div>
    </div>
  );
}

function callOut(call: TraceCall, tr: (k: string, v?: Record<string, unknown>) => string): string {
  if (call.outcome?.status === "error") return `${tr("system.error")}: ${call.outcome.message ?? call.outcome.error ?? ""}`;
  const tools = call.tools.map((tool) => `tool_use: ${tool.name}`).join(", ");
  return [clip(call.output.text, 80), tools].filter(Boolean).join(" · ") || "—";
}

interface CallDetailData {
  input: {
    system: { source?: string | null; origin?: unknown; text?: string }[];
    tools: { name: string; description?: string; input_schema?: unknown }[];
    messages: { role: string; content: unknown }[];
    model: string;
    max_tokens: number;
    thinking_mode?: unknown;
    provider?: string;
  };
  output: {
    text: string;
    thinking_chars: number;
    tool_uses: { id: string; name: string; input: unknown }[];
    outcome: unknown;
    stop_reason?: string | null;
    usage?: unknown;
    duration_ms?: number | null;
  };
}

/** One call in full, inside the right column: what went in, what came out, what changed. */
function CallDetail({ app, session, call, previous, onBack }: { app: App; session: SessionKey; call: TraceCall; previous: TraceCall | null; onBack: () => void }) {
  const tr = useT();
  const [side, setSide] = useState<"input" | "output">("input");
  const [data, setData] = useState<CallDetailData | null>(null);
  const [before, setBefore] = useState<CallDetailData | null>(null);
  const [compare, setCompare] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const target = { instance: session.instance, session_id: session.sessionId };
    void app
      .call<CallDetailData>("harnix.system.trace.call", { ...target, index: call.index })
      .then(setData)
      .catch((e) => setError(errorText(e)));
    if (previous)
      void app
        .call<CallDetailData>("harnix.system.trace.call", { ...target, index: previous.index })
        .then(setBefore)
        .catch(() => undefined);
  }, [app, session.instance, session.sessionId, call.index, previous?.index]);
  const copy = () => void navigator.clipboard?.writeText(JSON.stringify(data, null, 2)).catch(() => undefined);
  return (
    <div className="system-pane call-detail" data-testid="call-detail">
      <div className="cd-h">
        <button type="button" className="ib back" data-testid="call-back" title={tr("common.back")} onClick={onBack}>
          <Glyph name="chevronRight" className="glyph flip" />
        </button>
        <b>#{call.index + 1}</b>
        <span className="muted">{call.model}</span>
        <span className="sp" />
        {previous ? (
          <button type="button" className={`btn sm outline${compare ? " on" : ""}`} data-testid="call-compare" onClick={() => setCompare(!compare)}>
            {tr("system.compare")}
          </button>
        ) : null}
        <button type="button" className="btn sm outline" onClick={copy}>
          {tr("system.copyJson")}
        </button>
        <button type="button" className="btn sm outline" data-testid="call-locate" onClick={() => void locateInConversation(app, session, call.tools.at(0)?.id ?? null)}>
          {tr("system.locateInConversation")}
        </button>
      </div>
      <div className="cd-stats">
        {usageText(call.usage)} · {call.duration_ms != null ? `${(call.duration_ms / 1000).toFixed(1)}s` : "—"} · {call.stop_reason ?? call.outcome?.status}
      </div>
      <div className="stabs" role="tablist">
        {(["input", "output"] as const).map((s) => (
          <button type="button" role="tab" key={s} data-side={s} aria-selected={side === s} className={side === s ? "on" : ""} onClick={() => setSide(s)}>
            {tr(`system.side.${s}`)}
          </button>
        ))}
      </div>
      {error ? <div className="empty">{error}</div> : null}
      {data && compare && before ? <CallDiff before={before} after={data} /> : null}
      {data && side === "input" ? (
        <div data-testid="call-input">
          <div className="label">{tr("system.blocks", { count: data.input.system.length })}</div>
          {data.input.system.map((block, i) => (
            <Disclosure key={i} testId="call-block" state="ok" summary={<span className="hx-tool-name">{block.source ?? `#${i + 1}`}</span>} tail={`${(block.text ?? "").length} chars`}>
              <pre className="hx-pre">{block.text}</pre>
            </Disclosure>
          ))}
          <div className="label">{tr("system.toolsCount", { count: data.input.tools.length })}</div>
          <div className="note">{data.input.tools.map((tool) => tool.name).join(", ") || tr("request.noTools")}</div>
          <div className="label">{tr("system.messagesCount", { count: data.input.messages.length })}</div>
          {data.input.messages.map((message, i) => (
            <Disclosure key={i} testId="call-message" state="ok" summary={<><span className="hx-tool-name">{i}</span><span className="hx-tool-arg">{message.role}</span></>} tail={clip(messageText(message.content), 60)}>
              <pre className="hx-pre">{JSON.stringify(message.content, null, 2)}</pre>
            </Disclosure>
          ))}
          <div className="label">{tr("system.config")}</div>
          <pre className="hx-pre">{JSON.stringify({ model: data.input.model, max_tokens: data.input.max_tokens, thinking_mode: data.input.thinking_mode, provider: data.input.provider }, null, 2)}</pre>
        </div>
      ) : null}
      {data && side === "output" ? (
        <div data-testid="call-output">
          {data.output.text ? <div className="otext">{data.output.text}</div> : null}
          {data.output.thinking_chars ? <div className="note">{tr("system.thinking", { chars: data.output.thinking_chars })}</div> : null}
          {data.output.tool_uses.map((tool) => (
            <div key={tool.id}>
              <div className="label">tool_use · {tool.name}</div>
              <pre className="hx-pre">{JSON.stringify(tool.input, null, 2)}</pre>
            </div>
          ))}
          {call.tools.map((tool) => (
            <div key={tool.id} className={tool.is_error ? "bad" : ""}>
              <div className="label">
                tool_result · {tool.name} {tool.is_error ? `· ${tr("system.error")}` : ""}
              </div>
              <pre className="hx-pre scroll">{tool.result ?? tr("system.noResultYet")}</pre>
            </div>
          ))}
          <div className="label">{tr("system.end")}</div>
          <pre className="hx-pre">{JSON.stringify({ outcome: data.output.outcome, stop_reason: data.output.stop_reason, usage: data.output.usage, duration_ms: data.output.duration_ms }, null, 2)}</pre>
        </div>
      ) : null}
    </div>
  );
}

/** What differs from the call before: system blocks, tools, and how many messages. */
function CallDiff({ before, after }: { before: CallDetailData; after: CallDetailData }) {
  const tr = useT();
  const lines: { sign: "+" | "-" | "~"; text: string }[] = [];
  const blocks = (d: CallDetailData) => new Map(d.input.system.map((b, i) => [b.source ?? `#${i + 1}`, b.text ?? ""]));
  const was = blocks(before);
  const is = blocks(after);
  for (const [name, text] of is) {
    if (!was.has(name)) lines.push({ sign: "+", text: tr("system.diff.block", { label: name, chars: text.length }) });
    else if (was.get(name) !== text) lines.push({ sign: "~", text: tr("system.diff.changed", { label: name, from: was.get(name)!.length, to: text.length }) });
  }
  for (const [name, text] of was) if (!is.has(name)) lines.push({ sign: "-", text: tr("system.diff.block", { label: name, chars: text.length }) });
  const had = new Set(before.input.tools.map((tool) => tool.name));
  const has = new Set(after.input.tools.map((tool) => tool.name));
  for (const name of has) if (!had.has(name)) lines.push({ sign: "+", text: tr("system.diff.tool", { name }) });
  for (const name of had) if (!has.has(name)) lines.push({ sign: "-", text: tr("system.diff.tool", { name }) });
  const added = after.input.messages.length - before.input.messages.length;
  if (added) lines.push({ sign: added > 0 ? "+" : "-", text: tr("system.diff.messages", { count: Math.abs(added) }) });
  if (!lines.length) return <div className="diff none">{tr("system.diff.same")}</div>;
  return (
    <div className="diff" data-testid="system-diff">
      {lines.map((line, i) => (
        <div key={i} className={`line ${line.sign === "+" ? "add" : line.sign === "-" ? "del" : "mod"}`}>
          <span className="sign">{line.sign}</span>
          {line.text}
        </div>
      ))}
    </div>
  );
}
