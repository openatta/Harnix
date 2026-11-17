/**
 * A person's message, changed from its menu (`session.action` at `message`): rewritten in place
 * (`observability.messages.edit`), or taken out of the context from here on
 * (`observability.messages.drop`). The model sees the change from its next call; what it already
 * answered stays. The engine changes the message where it is, so the conversation is read again
 * (the hub tells every watcher to).
 *
 * The message is found by its turn: `harnix.session.outline` says where the person's message that
 * started it is, and how many messages the history had then. That count goes with the change, and
 * the engine refuses it when the history has changed since — a message is changed only as it was
 * seen. Not while a turn runs or waits: the engine refuses that too.
 */

import { useState } from "react";
import { errorText, type SessionOutline } from "@harnix/core";
import { DialogBody, DialogFoot, t, useT, type ActionTarget, type HostApi, type Registry } from "@harnix/react";

type MessageTarget = Extract<ActionTarget, { kind: "message" }>;

/** The backend space a call about the message claims: the one its interface space lists. */
function claimOf(r: Registry, space: string): string {
  return r.at("space").find((c) => c.id === space)?.sessions ?? space;
}

/** Why the message cannot be changed now — its session running or waiting on a turn; null when it can. */
export function messageBusy(host: HostApi, target: ActionTarget): string | null {
  const current = host.session.current();
  if (!current || current.instance !== target.session.instance || current.sessionId !== target.session.sessionId) return null;
  const state = host.session.context();
  const running = state?.turn != null || state?.session?.turn_state === "running" || (state?.queue.length ?? 0) > 0;
  return running ? t("obs.message.busy") : null;
}

/** Where the message is in the history, and how many messages the history has: from the outline. */
async function placeOf(host: HostApi, target: MessageTarget, space: string): Promise<{ at: number; total: number }> {
  const outline = await host.call<SessionOutline>("harnix.session.outline", { instance: target.session.instance, session_id: target.session.sessionId, space });
  const turn = outline?.turns?.find((x) => x.turn_id === target.turnId);
  if (!turn) throw new Error(t("obs.message.gone"));
  return { at: Number(turn.seq), total: Number(outline.total) };
}

/** The two items of a person's message's menu. */
export function registerMessageActions(r: Registry, host: HostApi, owner: string): void {
  const isMessage = (target: ActionTarget) => target.kind === "message";
  r.register(
    "session.action",
    {
      id: "observability.message.edit",
      at: ["message"],
      icon: "edit",
      label: () => t("obs.message.edit"),
      match: isMessage,
      disabled: (target) => messageBusy(host, target),
      run: async (target) => {
        if (target.kind !== "message") return;
        const done = await host.dialogs.open<boolean>((close) => <EditMessage r={r} host={host} target={target} close={close} />, {
          title: t("obs.message.editTitle"),
          wide: true,
        });
        if (done) host.shell.notice(t("obs.message.edited"));
      },
    },
    owner,
  );
  r.register(
    "session.action",
    {
      id: "observability.message.drop",
      at: ["message"],
      icon: "trash",
      label: () => t("obs.message.drop"),
      match: isMessage,
      disabled: (target) => messageBusy(host, target),
      run: async (target) => {
        if (target.kind !== "message") return;
        const sure = await host.dialogs.confirm(t("obs.message.dropAsk"), { title: t("obs.message.drop"), confirm: t("obs.message.dropConfirm"), danger: true });
        if (!sure) return;
        const space = claimOf(r, target.space);
        const { at, total } = await placeOf(host, target, space);
        await host.call("observability.messages.drop", { instance: target.session.instance, session_id: target.session.sessionId, space, at: [at], total });
        host.shell.notice(t("obs.message.dropped"));
      },
    },
    owner,
  );
}

/** The rewrite: the message's words to change, then the change, against the history as it was seen. */
function EditMessage({ r, host, target, close }: { r: Registry; host: HostApi; target: MessageTarget; close: (done: boolean | null) => void }) {
  const tr = useT();
  const [text, setText] = useState(target.text);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const changed = text.trim() !== "" && text !== target.text;
  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      const space = claimOf(r, target.space);
      const { at, total } = await placeOf(host, target, space);
      await host.call("observability.messages.edit", { instance: target.session.instance, session_id: target.session.sessionId, space, at, text, total });
      close(true);
    } catch (e) {
      setError(errorText(e));
      setSaving(false);
    }
  };
  return (
    <>
      <DialogBody>
        <div className="field">
          <textarea
            className="obs-message-text"
            data-testid="message-edit-text"
            aria-label={tr("obs.message.editTitle")}
            rows={6}
            value={text}
            onChange={(e) => setText(e.target.value)}
          />
          <div className="hint">{tr("obs.message.editNote")}</div>
          {error ? (
            <div className="hint err" data-testid="message-edit-error" role="alert">
              {error}
            </div>
          ) : null}
        </div>
      </DialogBody>
      <DialogFoot>
        <button type="button" className="btn outline" onClick={() => close(null)}>
          {tr("common.cancel")}
        </button>
        <button type="button" className="btn primary" data-testid="message-edit-save" disabled={!changed || saving} onClick={() => void save()}>
          {tr("obs.message.save")}
        </button>
      </DialogFoot>
    </>
  );
}
