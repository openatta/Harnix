/** The context tab: what the next call carries, and the messages. */

import { useEffect, useState } from "react";
import { Disclosure, clip, useT } from "@harnix/react";

import type { App, SessionKey } from "../../app.ts";
import { renameDialog } from "../../dialogs.tsx";
import { Glyph } from "../../glyph.tsx";
import { SYSTEM } from "../constants.ts";
import { Failure } from "../failure.tsx";
import { isPlainText, messageText, originLabel } from "../format.ts";
import type { Context, HistoryMessage } from "../types.ts";
import { act, type View } from "../view.ts";
import { Facts } from "./facts.tsx";

export function ContextTab({ app, view, session }: { app: App; view: View; session: SessionKey }) {
  const tr = useT();
  const [context, setContext] = useState<Context | null>(null);
  const [history, setHistory] = useState<{ messages: HistoryMessage[]; total: number } | null>(null);
  const [failure, setFailure] = useState<unknown>(null);
  const [round, setRound] = useState(0);
  const target = { instance: session.instance, session_id: session.sessionId };
  useEffect(() => {
    let alive = true;
    const load = async () => {
      try {
        const [ctx, first] = await Promise.all([
          app.call<Context>("harnix.system.context", target),
          app.call<{ total: number }>("session.history", { ...target, space: SYSTEM, offset: 0, limit: 1 }),
        ]);
        const offset = Math.max(0, first.total - 50);
        const page = await app.call<{ messages: HistoryMessage[]; total: number }>("session.history", { ...target, space: SYSTEM, offset, limit: 50 });
        if (!alive) return;
        setContext(ctx);
        setHistory({ messages: page.messages ?? [], total: page.total });
        setFailure(null);
      } catch (e) {
        if (alive) setFailure(e ?? "error");
      }
    };
    void load();
    const off = app.client.onNotify((method, params) => {
      if (method === "harnix.turn_settled" && params.session_id === session.sessionId) void load();
    });
    return () => {
      alive = false;
      off();
    };
  }, [app, session.instance, session.sessionId, round]);
  if (failure) return <Failure error={failure} testId="system-context-failed" />;
  if (!context) return null;
  const total = history?.total ?? 0;
  const again = () => setRound((n) => n + 1);
  const systemTokens = (context.system ?? []).reduce((n, b) => n + b.tokens, 0);
  const toolTokens = (context.tools ?? []).reduce((n, b) => n + (b.tokens ?? 0), 0);
  const messageTokens = context.messages?.tokens ?? 0;
  // The bar is as long as the provider's own count of the last call (when there is one), against the
  // window; the estimates only divide it. An estimate is never shown as if it were a count: `~`.
  const estimated = Math.max(systemTokens + toolTokens + messageTokens, 1);
  const measured = context.measured?.tokens ?? null;
  const threshold = context.measured?.compact_threshold ?? context.compact_threshold ?? 0;
  const length = measured ?? estimated;
  const budget = Math.max(context.measured?.window ?? 0, threshold, length, 1);
  const pct = (n: number) => `${Math.min(100, ((n / estimated) * length * 100) / budget)}%`;
  const about = (n: number) => `~${n}`;
  return (
    <div className="system-pane" data-testid="system-context">
      <div className="note">{tr("system.contextNote")}</div>
      <div className="stack" data-testid="context-stack" data-measured={measured ?? undefined} title={measured != null ? tr("system.tokens", { count: measured }) : tr("system.tokens", { count: about(estimated) })}>
        <i className="sys" style={{ width: pct(systemTokens) }} />
        <i className="tools" style={{ width: pct(toolTokens) }} />
        <i className="msgs" style={{ width: pct(messageTokens) }} />
      </div>
      <div className="legend2">
        <span className="sys">{tr("system.legend.system", { count: about(systemTokens) })}</span>
        <span className="tools">{tr("system.legend.tools", { count: about(toolTokens) })}</span>
        <span className="msgs">{tr("system.legend.messages", { count: about(messageTokens) })}</span>
      </div>
      <Facts
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
      <div className="label">{tr("system.messages")}</div>
      <div className="messages" data-testid="system-messages">
        {(history?.messages ?? []).map((message) => {
          const origin = message.origin ?? message.role;
          const text = messageText(message.content);
          const droppable = origin === "user" || origin === "steer" || origin === "command" || origin === "tool_result";
          const editable = (origin === "user" || origin === "steer") && isPlainText(message.content);
          return (
            <div key={message.seq} className="message" data-seq={message.seq} data-origin={origin}>
              <span className="seq">{message.seq}</span>
              <span className="origin">{origin}</span>
              <span className="text truncate" title={text}>
                {clip(text, 160)}
              </span>
              {editable ? (
                <button
                  type="button"
                  className="icon-btn"
                  data-testid="message-edit"
                  title={tr("system.editMessage")}
                  onClick={() =>
                    renameDialog(app, tr("system.editMessage"), text, async (value) => {
                      await app.call("harnix.system.messages.edit", { ...target, at: message.seq, text: value, total });
                      again();
                    })
                  }
                >
                  <Glyph name="edit" className="glyph" />
                </button>
              ) : null}
              <button
                type="button"
                className="icon-btn"
                data-testid="message-fork"
                title={tr("system.forkHere")}
                onClick={() =>
                  void act(app, async () => {
                    // `at_message` is how many to keep: this one and everything before it.
                    const forked = await app.call<{ session_id: string; instance?: string }>("session.fork", { ...target, at_message: message.seq + 1, space: SYSTEM });
                    void app.refreshSessions();
                    view.select({ kind: "session", instance: forked.instance ?? session.instance, sessionId: forked.session_id });
                  })
                }
              >
                <Glyph name="fork" className="glyph" />
              </button>
              {droppable ? (
                <button
                  type="button"
                  className="icon-btn"
                  data-testid="message-drop"
                  title={tr("system.dropMessage")}
                  onClick={() =>
                    void act(app, async () => {
                      await app.call("harnix.system.messages.drop", { ...target, at: [message.seq], total });
                      again();
                    })
                  }
                >
                  <Glyph name="trash" className="glyph" />
                </button>
              ) : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}
