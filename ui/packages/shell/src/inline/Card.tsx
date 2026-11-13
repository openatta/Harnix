/**
 * `flow.block`, declared: an entry an application appended to a session (`harnix.session.entries.append`,
 * whose event names the kind) drawn as the card its manifest's `[[inline.flow_block]]` describes —
 * a title, key and value rows, a Markdown body, and buttons that call the application's own
 * methods. A button that says `confirm` asks through the shell's own dialog first; what the method
 * answers is shown in the card (`done`).
 *
 * The card says which application it is from: words in the flow that are not the shell's own are
 * marked as the application's.
 */

import { Fragment, useState } from "react";
import { errorText } from "@harnix/core";
import { markdown, useT, Glyph } from "@harnix/react";

import type { App } from "../app.ts";
import { confirmDialog } from "../dialogs.tsx";
import type { ButtonDecl, FlowBlockDecl, RuntimePackage } from "./packages.ts";
import { fill, fillValue, say } from "./template.ts";

/** What an entry is, as the session's log carries it. */
export interface Entry {
  id?: string;
  ns?: string;
  event?: string;
  payload?: unknown;
}

export function InlineCard({ app, owner, decl, entry }: { app: App; owner: RuntimePackage; decl: FlowBlockDecl; entry: Entry }) {
  const t = useT();
  const scope = { payload: entry.payload ?? {} };
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async (button: ButtonDecl) => {
    setBusy(true);
    try {
      const result = await app.call(`${owner.id}.${button.method}`, fillValue(button.params ?? {}, scope));
      setDone(button.done ? say(button.done, { ...scope, result }) : t("packages.command.done", { command: say(button.label, scope) }));
      setError(null);
    } catch (e) {
      setError(t("packages.card.failed", { name: owner.title, error: errorText(e) }));
    } finally {
      setBusy(false);
    }
  };
  const press = (button: ButtonDecl) => {
    if (!button.confirm) return void run(button);
    confirmDialog(app, say(button.confirm, scope), async () => {
      app.closeModal();
      await run(button);
    });
  };

  const fields = decl.field ?? [];
  const buttons = decl.button ?? [];
  return (
    <div className="hx-blk package-card" data-testid="package-card" data-package={owner.id} data-kind={decl.kind} data-entry={entry.id}>
      <div className="package-card-head">
        <Glyph name={decl.icon ?? "file"} className="package-inline-icon" />
        <span className="package-card-title" data-testid="package-card-title">
          {say(decl.title, scope)}
        </span>
        <span className="package-card-from">{t("packages.card.from", { name: owner.title })}</span>
      </div>
      {fields.length ? (
        <dl className="package-card-fields">
          {fields.map((field, i) => (
            <Fragment key={i}>
              <dt>{say(field.label, scope)}</dt>
              <dd data-testid="package-card-field">{fill(field.value, scope)}</dd>
            </Fragment>
          ))}
        </dl>
      ) : null}
      {decl.body ? <div className="hx-md package-card-body" data-testid="package-card-body" dangerouslySetInnerHTML={{ __html: markdown(say(decl.body, scope)) }} /> : null}
      {buttons.length ? (
        <div className="actions">
          {buttons.map((button, i) => (
            <button key={i} type="button" className="btn sm outline" data-testid="package-card-button" data-method={button.method} disabled={busy} onClick={() => press(button)}>
              {say(button.label, scope)}
            </button>
          ))}
        </div>
      ) : null}
      {done !== null ? (
        <div className="package-card-done" role="status" data-testid="package-card-done">
          {done}
        </div>
      ) : null}
      {error !== null ? (
        <div className="hint err" role="alert" data-testid="package-card-error">
          {error}
        </div>
      ) : null}
    </div>
  );
}
