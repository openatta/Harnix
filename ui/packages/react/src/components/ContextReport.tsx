/**
 * What a session's next model call carries, drawn in the right column's context tab
 * (`@harnix/observability`'s). The bar is as long as the provider's own count of the last call,
 * against the window; the estimates only divide it, and an estimate is never shown as if it were a
 * count (`~`).
 */

import { Disclosure } from "./blocks";
import { useT } from "../i18n/index";

/** `observability.context`'s answer (the engine's `inspectContext`, with `measured` beside it). */
export interface ContextReportData {
  model?: string;
  provider?: string;
  max_tokens?: number;
  system?: { name: string; origin: unknown; tokens: number; text: string }[];
  tools?: { name: string; source?: string; tokens?: number }[];
  messages?: { count: number; tokens: number };
  total_tokens?: number;
  compact_threshold?: number;
  /** What the provider counted for the last call made; absent before the first. */
  measured?: { tokens: number; window?: number; compact_threshold?: number };
}

/** Who put a system block there, as the engine says it: a word, or `kind:value`. */
export function originLabel(origin: unknown): string {
  if (typeof origin === "string") return origin;
  if (origin && typeof origin === "object") {
    const [kind, value] = Object.entries(origin as Record<string, unknown>)[0] ?? [];
    return kind ? `${kind}:${String(value)}` : "";
  }
  return "";
}

/** Facts, a label and its value each; the empty ones left out. */
export function FactList({ rows, testId }: { rows: [string, string][]; testId?: string }) {
  return (
    <dl className="hx-facts" data-testid={testId}>
      {rows
        .filter(([, value]) => value)
        .map(([label, value]) => (
          <div key={label} className="hx-fact">
            <dt>{label}</dt>
            <dd title={value}>{value}</dd>
          </div>
        ))}
    </dl>
  );
}

export function ContextReport({ context, blocks = true }: { context: ContextReportData; blocks?: boolean }) {
  const tr = useT();
  const systemTokens = (context.system ?? []).reduce((n, b) => n + b.tokens, 0);
  const toolTokens = (context.tools ?? []).reduce((n, b) => n + (b.tokens ?? 0), 0);
  const messageTokens = context.messages?.tokens ?? 0;
  const estimated = Math.max(systemTokens + toolTokens + messageTokens, 1);
  const measured = context.measured?.tokens ?? null;
  const threshold = context.measured?.compact_threshold ?? context.compact_threshold ?? 0;
  const length = measured ?? estimated;
  const budget = Math.max(context.measured?.window ?? 0, threshold, length, 1);
  const pct = (n: number) => `${Math.min(100, ((n / estimated) * length * 100) / budget)}%`;
  const about = (n: number) => `~${n}`;
  return (
    <div className="hx-ctx" data-testid="context-report">
      <div
        className="hx-ctx-stack stack"
        data-testid="context-stack"
        data-measured={measured ?? undefined}
        title={measured != null ? tr("system.tokens", { count: measured }) : tr("system.tokens", { count: about(estimated) })}
      >
        <i className="sys" style={{ width: pct(systemTokens) }} />
        <i className="tools" style={{ width: pct(toolTokens) }} />
        <i className="msgs" style={{ width: pct(messageTokens) }} />
      </div>
      <div className="hx-ctx-legend legend2">
        <span className="sys">{tr("system.legend.system", { count: about(systemTokens) })}</span>
        <span className="tools">{tr("system.legend.tools", { count: about(toolTokens) })}</span>
        <span className="msgs">{tr("system.legend.messages", { count: about(messageTokens) })}</span>
      </div>
      <FactList
        rows={[
          [tr("system.fact.model"), context.model ?? ""],
          [
            tr("system.fact.measured"),
            measured != null
              ? context.measured?.window
                ? tr("system.tokensOf", { count: measured, of: context.measured.window })
                : tr("system.tokens", { count: measured })
              : tr("system.notMeasured"),
          ],
          [tr("system.fact.estimated"), tr("system.tokens", { count: about(context.total_tokens ?? estimated) })],
          [tr("system.fact.threshold"), threshold ? tr("system.tokens", { count: threshold }) : ""],
          [tr("system.fact.messages"), tr("system.messagesTokens", { count: context.messages?.count ?? 0, tokens: about(context.messages?.tokens ?? 0) })],
        ]}
      />
      {blocks ? (
        <>
          <div className="label">{tr("system.blocks", { count: context.system?.length ?? 0 })}</div>
          {(context.system ?? []).map((block, i) => (
            <Disclosure
              key={i}
              testId="system-block"
              state="ok"
              summary={
                <>
                  <span className="hx-tool-name">{block.name}</span>
                  <span className="hx-tool-arg">{originLabel(block.origin)}</span>
                </>
              }
              tail={tr("system.tokens", { count: about(block.tokens) })}
            >
              <pre className="hx-pre">{block.text}</pre>
            </Disclosure>
          ))}
          <div className="label">{tr("system.toolsCount", { count: context.tools?.length ?? 0 })}</div>
          <div className="note">{(context.tools ?? []).map((tool) => tool.name).join(", ") || tr("request.noTools")}</div>
        </>
      ) : null}
    </div>
  );
}
