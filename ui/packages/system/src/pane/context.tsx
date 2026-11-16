/** The context tab: what the next call carries, and the messages. */

import { useEffect, useState } from "react";
import { ContextReport, clip, useT, Glyph, type HostSessionRef as SessionKey } from "@harnix/react";

import { SYSTEM } from "../constants.ts";
import { Failure } from "../failure.tsx";
import { isPlainText, messageText } from "../format.ts";
import type { Context, HistoryMessage } from "../types.ts";
import { act, type View } from "../view.ts";

export function ContextTab({ view, session }: { view: View; session: SessionKey }) {
  const host = view.host;
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
          host.call<Context>("system.context", target),
          host.call<{ total: number }>("engine.session.history", { ...target, space: SYSTEM, offset: 0, limit: 1 }),
        ]);
        const offset = Math.max(0, first.total - 50);
        const page = await host.call<{ messages: HistoryMessage[]; total: number }>("engine.session.history", { ...target, space: SYSTEM, offset, limit: 50 });
        if (!alive) return;
        setContext(ctx);
        setHistory({ messages: page.messages ?? [], total: page.total });
        setFailure(null);
      } catch (e) {
        if (alive) setFailure(e ?? "error");
      }
    };
    void load();
    const off = host.events.onTurnSettled((e) => {
      if (e.session_id === session.sessionId) void load();
    });
    return () => {
      alive = false;
      off();
    };
  }, [host, session.instance, session.sessionId, round]);
  if (failure) return <Failure error={failure} testId="system-context-failed" />;
  if (!context) return null;
  const total = history?.total ?? 0;
  const again = () => setRound((n) => n + 1);
  return (
    <div className="system-pane" data-testid="system-context">
      <div className="note">{tr("system.contextNote")}</div>
      <ContextReport context={context} />
      <div className="label">{tr("system.messages")}</div>
      <div className="messages" data-testid="system-messages">
        {(history?.messages ?? []).map((message) => {
          const origin = message.origin ?? message.role;
          const text = messageText(message.content);
          const droppable = origin === "user" || origin === "steer" || origin === "command" || origin === "tool_result";
          const editable = (origin === "user" || origin === "steer") && isPlainText(message.content);
          return (
            <div key={message.seq} className="message" data-testid="system-message" data-seq={message.seq} data-origin={origin}>
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
                    void host.dialogs.prompt({ title: tr("system.editMessage"), label: tr("system.editMessage"), value: text }).then((value) => {
                      if (value === null) return;
                      void act(host, async () => {
                        await host.call("system.messages.edit", { ...target, at: message.seq, text: value, total });
                        again();
                      });
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
                  void act(host, async () => {
                    // `at_message` is how many to keep: this one and everything before it.
                    const forked = await host.call<{ session_id: string; instance?: string }>("harnix.session.fork", { ...target, at_message: message.seq + 1, space: SYSTEM });
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
                    void act(host, async () => {
                      await host.call("system.messages.drop", { ...target, at: [message.seq], total });
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
