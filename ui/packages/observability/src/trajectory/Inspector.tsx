/** The inspector inside the trajectory: one call in full — what went in, what came out, what changed since the call before. */

import { useEffect, useState } from "react";
import { Disclosure, clip, useT, Glyph, type HostSessionRef as SessionKey } from "@harnix/react";

import { Failure, locateInConversation, messageText, sessionParams, usageText, type Observed, type TraceCall } from "../shared.tsx";
import { failed } from "./model.ts";

interface CallDetailData {
  input: {
    system: { source?: string | null; origin?: unknown; text?: string }[];
    tools: { name: string; description?: string; input_schema?: unknown }[];
    messages: { role: string; content: unknown }[];
    model: string;
    max_tokens: number;
    thinking_mode?: unknown;
    provider?: string;
    effort?: unknown;
  };
  output: {
    text: string;
    thinking_chars: number;
    tool_uses: { id: string; name: string; input: unknown }[];
    /** What each tool call got back, as the session's next call sent it; empty until that call is made. */
    tool_results?: { id: string; text: string; chars: number; is_error: boolean }[];
    status?: string;
    error?: unknown;
    stop_reason?: string | null;
    usage?: unknown;
    duration_ms?: number | null;
  };
}

export function CallDetail({
  observed,
  session,
  call,
  ordinal,
  previous,
  onBack,
}: {
  observed: Observed;
  session: SessionKey;
  call: TraceCall;
  ordinal: number;
  previous: TraceCall | null;
  onBack: () => void;
}) {
  const tr = useT();
  const [side, setSide] = useState<"input" | "output">("input");
  const [data, setData] = useState<CallDetailData | null>(null);
  const [before, setBefore] = useState<CallDetailData | null>(null);
  const [compare, setCompare] = useState(false);
  const [error, setError] = useState<unknown>(null);
  useEffect(() => {
    let alive = true;
    setData(null);
    setBefore(null);
    setError(null);
    const target = sessionParams(observed, session);
    void observed.host
      .call<CallDetailData>("observability.trace.call", { ...target, index: call.index })
      .then((d) => alive && setData(d))
      .catch((e) => alive && setError(e ?? "error"));
    if (previous)
      void observed.host
        .call<CallDetailData>("observability.trace.call", { ...target, index: previous.index })
        .then((d) => alive && setBefore(d))
        .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [observed, session.instance, session.sessionId, call.index, previous?.index]);
  const copy = () => void navigator.clipboard?.writeText(JSON.stringify(data, null, 2)).catch(() => undefined);
  return (
    <div className="traj-inspector call-detail" data-testid="call-detail" data-call={call.index}>
      <div className="cd-h">
        <button type="button" className="ib back" data-testid="call-back" title={tr("common.back")} onClick={onBack}>
          <Glyph name="close" className="glyph" />
        </button>
        <b>#{ordinal}</b>
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
        <button type="button" className="btn sm outline" data-testid="call-locate" onClick={() => void locateInConversation(observed.host, session, call.tools.at(0)?.id ?? null)}>
          {tr("system.locateInConversation")}
        </button>
      </div>
      <div className="cd-stats">
        {usageText(call.usage)} · {call.duration_ms != null ? `${(call.duration_ms / 1000).toFixed(1)}s` : "—"} · {call.stop_reason ?? call.status}
        {/* The engine leaves `message` empty when the provider said nothing more than the kind. */}
        {failed(call) ? <span data-testid="call-failure"> · {call.error?.message || call.error?.kind}</span> : null}
      </div>
      <div className="stabs" role="tablist">
        {(["input", "output"] as const).map((s) => (
          <button type="button" role="tab" key={s} data-side={s} aria-selected={side === s} className={side === s ? "on" : ""} onClick={() => setSide(s)}>
            {tr(`system.side.${s}`)}
          </button>
        ))}
      </div>
      {error ? <Failure error={error} /> : null}
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
            <Disclosure
              key={i}
              testId="call-message"
              state="ok"
              summary={
                <>
                  <span className="hx-tool-name">{i}</span>
                  <span className="hx-tool-arg">{message.role}</span>
                </>
              }
              tail={clip(messageText(message.content), 60)}
            >
              <pre className="hx-pre">{JSON.stringify(message.content, null, 2)}</pre>
            </Disclosure>
          ))}
          <div className="label">{tr("system.config")}</div>
          <pre className="hx-pre">{JSON.stringify({ model: data.input.model, max_tokens: data.input.max_tokens, thinking_mode: data.input.thinking_mode, effort: data.input.effort, provider: data.input.provider }, null, 2)}</pre>
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
          {data.output.tool_uses.map((tool) => {
            const result = data.output.tool_results?.find((r) => r.id === tool.id);
            return (
              <div key={tool.id} className={result?.is_error ? "bad" : ""}>
                <div className="label">
                  tool_result · {tool.name} {result?.is_error ? `· ${tr("system.error")}` : ""}
                </div>
                <pre className="hx-pre scroll" data-testid="call-tool-result">
                  {result ? result.text : tr("system.noResultYet")}
                </pre>
              </div>
            );
          })}
          <div className="label">{tr("system.end")}</div>
          <pre className="hx-pre">{JSON.stringify({ status: data.output.status, error: data.output.error, stop_reason: data.output.stop_reason, usage: data.output.usage, duration_ms: data.output.duration_ms }, null, 2)}</pre>
        </div>
      ) : null}
    </div>
  );
}

/** What differs from the call before: system blocks, tools, and how many messages. */
export function CallDiff({ before, after }: { before: CallDetailData; after: CallDetailData }) {
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
