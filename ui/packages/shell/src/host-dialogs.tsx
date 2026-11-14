/**
 * `host.dialogs`: the shell's own layer for what a UI package asks — a confirmation, one line of
 * text, a dialog of its own, a menu. Focus, Escape and the page made inert under it are the
 * shell's, so a package makes none of its own. Each settles once: with the answer, or with
 * nothing (`false`, `null`) when the reader closed it another way — Escape, the veil, another
 * dialog taking its place.
 */

import { useEffect, useRef, useState, type ReactNode } from "react";
import { t, useT, type HostApi } from "@harnix/react";

import type { App } from "./app.ts";
import { Body, Foot } from "./dialogs.tsx";
import { menuBelow, openMenu } from "./Menu.tsx";

/** Calls `onGone` when it leaves the page. */
function Gone({ onGone, children }: { onGone: () => void; children: ReactNode }) {
  return (
    <div
      className="hx-host-dialog"
      ref={(el) => {
        if (!el) onGone();
      }}
    >
      {children}
    </div>
  );
}

/** Open a dialog that settles with what it closed with, or `dismissed` when it went otherwise. */
function settled<T>(app: App, title: string, wide: boolean | undefined, dismissed: T, body: (close: (result: T) => void) => ReactNode): Promise<T> {
  return new Promise((resolve) => {
    let done = false;
    const settle = (value: T) => {
      if (done) return;
      done = true;
      resolve(value);
    };
    const close = (result: T) => {
      settle(result);
      app.closeModal();
    };
    app.openModal({ title, wide, content: <Gone onGone={() => settle(dismissed)}>{body(close)}</Gone> });
  });
}

function Prompt({ label, value, validate, close }: { label: string; value: string; validate?: (v: string) => string | null; close: (v: string | null) => void }) {
  const t = useT();
  const [text, setText] = useState(value);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus(), []);
  let problem: string | null = null;
  try {
    problem = validate?.(text) ?? null;
  } catch (e) {
    problem = e instanceof Error ? e.message : String(e);
  }
  const commit = () => {
    if (problem === null) close(text);
  };
  return (
    <>
      <Body>
        <div className="field">
          <label>{label}</label>
          <input
            ref={input}
            type="text"
            value={text}
            data-testid="host-prompt-input"
            aria-invalid={problem !== null || undefined}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key !== "Enter") return;
              // The key is the prompt's: closing hands the focus back to the button that opened it,
              // which would take the same Enter and open it again.
              e.preventDefault();
              commit();
            }}
          />
          {problem ? <div className="hint err">{problem}</div> : null}
        </div>
      </Body>
      <Foot>
        <button type="button" className="btn outline" onClick={() => close(null)}>
          {t("common.cancel")}
        </button>
        <button type="button" className="btn primary" data-testid="host-prompt-ok" disabled={problem !== null} onClick={commit}>
          {t("common.confirm")}
        </button>
      </Foot>
    </>
  );
}

export function hostDialogs(app: App): HostApi["dialogs"] {
  return {
    confirm(text, opts = {}) {
      return settled(app, opts.title ?? t("dialog.confirm"), false, false, (close) => (
        <>
          <Body>
            <div>{text}</div>
          </Body>
          <Foot>
            <button type="button" className="btn outline" data-testid="confirm-cancel" onClick={() => close(false)}>
              {t("common.cancel")}
            </button>
            <button type="button" className={`btn ${opts.danger ? "danger" : "primary"}`} data-testid="confirm" onClick={() => close(true)}>
              {opts.confirm ?? t("common.confirm")}
            </button>
          </Foot>
        </>
      ));
    },
    prompt(opts) {
      return settled<string | null>(app, opts.title, false, null, (close) => <Prompt label={opts.label} value={opts.value ?? ""} validate={opts.validate} close={close} />);
    },
    open<T>(render: (close: (result: T | null) => void) => ReactNode, opts: { title?: string; wide?: boolean } = {}) {
      // Drawn as given: the dialog lays out its own body and buttons (`DialogBody`, `DialogFoot`).
      return settled<T | null>(app, opts.title ?? "", opts.wide, null, (close) => render(close));
    },
    menu(anchor, items) {
      const at = anchor instanceof Element ? menuBelow(anchor) : anchor;
      openMenu({
        label: t("host.menu"),
        at,
        returnTo: anchor instanceof HTMLElement ? anchor : null,
        testId: "host-menu",
        items: items.map((item) => ({
          id: item.id,
          label: item.label,
          icon: item.icon,
          danger: item.danger,
          disabled: item.disabled,
          run: () => {
            void Promise.resolve()
              .then(() => item.run())
              .catch((e: unknown) => app.notice(e instanceof Error ? e.message : String(e), "error"));
          },
        })),
      });
      return Promise.resolve();
    },
  };
}
