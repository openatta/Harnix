/**
 * The request envelope: what the model was sent minus the messages — the assembled system blocks,
 * the tool catalog, the call configuration (§6.4). `harnix.requestHeaders` folds the engine's
 * recording down to the points where it changed; each change becomes a row in the flow (a `custom`
 * block of type `request`), and the details pane shows it at full length.
 *
 * Read when a session has been opened and after each live `turn_complete`. A diagnostic that cannot
 * be read is no reason to disturb a conversation that otherwise works, so failures are swallowed.
 */

import type { SessionController } from "@harnix/core/state";
import { Disclosure, clip, t } from "@harnix/react";

import type { App } from "./app.ts";
import { Glyph } from "./glyph.tsx";

export interface SystemPart {
  role?: string;
  cache?: string | null;
  source?: string | null;
  text?: string;
}

export interface ToolEntry {
  name: string;
  description?: string;
  input_schema?: unknown;
  source?: string | null;
}

export interface Header {
  model?: string;
  max_tokens?: number;
  thinking_mode?: unknown;
  provider?: string | null;
  turn?: number | null;
  step?: number | null;
  system?: SystemPart[];
  tools?: ToolEntry[];
}

export type Change = "initial" | "system" | "tools" | "systemAndTools" | "config";

export interface RequestBlock {
  header: Header;
  previous: Header | null;
  change: Change;
}

/** Follow one opened session's envelopes. Returns the unsubscribe. */
export function followEnvelopes(app: App, controller: SessionController): () => void {
  let seen: string[] = [];
  let previous: Header | null = null;
  let wasLoading = true;
  let reading = false;

  const read = async () => {
    if (reading) return;
    reading = true;
    try {
      const answer = await app.call<{ headers?: Header[] }>("harnix.requestHeaders", {
        instance: controller.instance,
        session_id: controller.sessionId,
      });
      const headers = answer.headers ?? [];
      let shown = 0;
      while (shown < headers.length && shown < seen.length && seen[shown] === signature(headers[shown]!)) shown += 1;
      const fresh = headers.slice(shown);
      seen = headers.map(signature);
      for (const header of fresh) {
        if (!controller.insert("request", { header, previous, change: change(previous, header) } satisfies RequestBlock)) break;
        previous = header;
      }
    } catch {
      /* see the file comment */
    } finally {
      reading = false;
    }
  };

  const offState = controller.subscribe(() => {
    const loading = controller.getSnapshot().loading;
    if (loading && !wasLoading) {
      // Opened again (a reconnect, a resync): the flow is rebuilt, so every envelope goes back in.
      seen = [];
      previous = null;
    }
    if (!loading && wasLoading) void read();
    wasLoading = loading;
  });
  const offNotify = app.client.onNotify((method, params) => {
    if (method !== "harnix.event" || params.session_id !== controller.sessionId) return;
    if ((params.instance ?? "system") !== controller.instance) return;
    const kind = (params.event as { kind?: string } | undefined)?.kind;
    if (kind === "turn_complete") void read();
  });
  if (!controller.getSnapshot().loading) {
    wasLoading = false;
    void read();
  }
  return () => {
    offState();
    offNotify();
  };
}

/** What moved between two envelopes. */
export function change(previous: Header | null, header: Header): Change {
  if (!previous) return "initial";
  const sameSystem = systemSignature(previous) === systemSignature(header);
  const sameTools = toolSignature(previous) === toolSignature(header);
  if (!sameSystem && !sameTools) return "systemAndTools";
  if (!sameSystem) return "system";
  if (!sameTools) return "tools";
  // The hub reports only envelopes that differ from the one before, so what is left is the
  // call configuration: a model fallback, a token ceiling, a thinking-mode switch.
  return "config";
}

/* Joined on characters that cannot occur inside a field, so two different envelopes cannot agree
 * by accident. */
function systemSignature(header: Header): string {
  return (header.system ?? []).map((b) => `${b.role}\u0000${b.cache ?? ""}\u0000${b.text ?? ""}`).join("\u0001");
}

function toolSignature(header: Header): string {
  return JSON.stringify(header.tools ?? []);
}

function signature(header: Header): string {
  return [header.model, header.max_tokens, JSON.stringify(header.thinking_mode), systemSignature(header), toolSignature(header)].join("\u0000");
}

/* ── formatting ───────────────────────────────────────────────────────── */

const systemBlocks = (h: Header) => h.system ?? [];
const tools = (h: Header) => h.tools ?? [];
const systemBytes = (h: Header) => systemBlocks(h).reduce((n, b) => n + (b.text ?? "").length, 0);

export function sizeSummary(h: Header): string {
  return t("request.size", { kb: (systemBytes(h) / 1024).toFixed(1), tools: tools(h).length });
}

function thinkingLabel(mode: unknown): string {
  if (mode == null) return "?";
  if (typeof mode === "string") return mode;
  const [name, value] = Object.entries(mode as Record<string, unknown>)[0] ?? [];
  return value === undefined ? String(name) : `${name} ${JSON.stringify(value)}`;
}

function configLine(h: Header): string {
  return t("request.configLine", { model: h.model ?? "?", max: h.max_tokens ?? 0, thinking: thinkingLabel(h.thinking_mode) });
}

/* The source, when the recording has one, replaces the role: every block is a system block, and
 * which stage assembled it is what a numbered list cannot say. */
function systemLabel(block: SystemPart, index: number): string {
  const head = block.source
    ? t("request.systemBlockSource", { n: index + 1, source: block.source })
    : t("request.systemBlock", { n: index + 1, role: block.role ?? "system" });
  return block.cache ? `${head} · ${t("request.cache", { marker: block.cache })}` : head;
}

function toolNames(h: Header): string {
  const names = tools(h).map((tool) => tool.name);
  return names.length ? names.join(", ") : t("request.noTools");
}

/* A per-source tally rather than the list: a changed catalog is usually an MCP server connecting,
 * and "28 builtin · 3 mcp:files" says that where thirty names say nothing. */
function toolSources(h: Header): string {
  const tally = new Map<string, number>();
  for (const tool of tools(h)) {
    const source = tool.source ?? t("request.sourceUnknown");
    tally.set(source, (tally.get(source) ?? 0) + 1);
  }
  return [...tally].map(([source, count]) => `${count} ${source}`).join(" · ");
}

function toolLine(tool: ToolEntry): string {
  const head = tool.source ? `${tool.name} · ${tool.source}` : tool.name;
  return tool.description ? `${head} — ${tool.description}` : head;
}

export function Labelled({ label, children, testId }: { label: string; children: string; testId?: string }) {
  return (
    <div className="hx-labelled" data-testid={testId}>
      <div className="hx-label label">{label}</div>
      <pre className="hx-pre">{children}</pre>
    </div>
  );
}

/* ── the flow row ─────────────────────────────────────────────────────── */

export function RequestRow({ app, block }: { app: App; block: RequestBlock }) {
  const header = block.header ?? {};
  const sources = toolSources(header);
  return (
    <Disclosure
      testId="conv-request"
      state="ok"
      summary={
        <>
          <Glyph name="envelope" className="lead" />
          <span className="hx-tool-name name">{t("request.title")}</span>
          <span className="hx-tool-arg summary">{t(`request.change.${block.change}`)}</span>
        </>
      }
      tail={sizeSummary(header)}
    >
      <Labelled label={t("request.config")}>{configLine(header)}</Labelled>
      {systemBlocks(header).map((part, i) => (
        <Labelled key={i} label={systemLabel(part, i)}>
          {clip(part.text ?? "", 4000)}
        </Labelled>
      ))}
      <Labelled label={t("request.tools", { count: tools(header).length })}>{toolNames(header)}</Labelled>
      {sources ? <Labelled label={t("request.toolSources")}>{sources}</Labelled> : null}
      <div className="actions">
        <button
          type="button"
          className="btn sm outline"
          data-testid="open-in-details"
          onClick={() => app.showDetail({ kind: "request", name: t("request.title"), ...block })}
        >
          {t("flow.openInDetails")}
        </button>
      </div>
    </Disclosure>
  );
}

/* ── the details panel ────────────────────────────────────────────────── */

/* Uncut on purpose: this pane exists for the reader who needs the exact text. The flow row clips. */
export function RequestPanel({ block }: { block: RequestBlock }) {
  const header = block.header ?? {};
  const catalog = tools(header);
  return (
    <div data-testid="details-request">
      <div className="panel-head">
        <Glyph name="envelope" className="lead" />
        <span>{t("request.title")}</span>
      </div>
      <div className="note">{t("request.inForce")}</div>
      <Labelled label={t("request.config")}>{configLine(header)}</Labelled>
      {header.provider ? <Labelled label={t("request.provider")}>{header.provider}</Labelled> : null}
      <Labelled label={t("request.at")}>
        {t("request.turnStep", { turn: header.turn ?? "?", step: header.step ?? "?" })}
      </Labelled>
      {systemBlocks(header).map((part, i) => (
        <Labelled key={i} label={systemLabel(part, i)}>
          {part.text ?? ""}
        </Labelled>
      ))}
      <Labelled label={t("request.tools", { count: catalog.length })}>
        {catalog.map(toolLine).join("\n") || t("request.noTools")}
      </Labelled>
      {catalog.length ? (
        <Labelled label={t("request.schemas")}>
          {JSON.stringify(Object.fromEntries(catalog.map((tool) => [tool.name, tool.input_schema])), null, 2)}
        </Labelled>
      ) : null}
      {block.previous && (block.change === "system" || block.change === "systemAndTools") ? (
        <Labelled label={t("request.previousSystem")}>
          {systemBlocks(block.previous).map((part) => part.text ?? "").join("\n\n")}
        </Labelled>
      ) : null}
    </div>
  );
}
