/**
 * The shell's own question to the person, on an application's behalf: `confirm` and `openLink`
 * over the bridge. Drawn by the page, over everything — every frame, and the shell's own dialog
 * if one is open (a settings section's frame may ask) — so no application can draw it, cover it
 * or answer it: its frames are inert while it is up, and it names who is asking.
 *
 * Keys are its while it is open: Escape answers no, and nothing else on the page sees it — the
 * listener is the page's first (registered when this module loads, before the shell's), which is
 * what keeps an Escape here from also closing the dialog underneath (ipdrun-38).
 */

import { useEffect, useRef, useSyncExternalStore } from "react";
import { useT } from "@harnix/react";

export interface Question {
  /** The application asking, by its title. */
  app: string;
  kind: "confirm" | "link";
  text: string;
  title?: string;
  confirm?: string;
  danger?: boolean;
}

interface Asked extends Question {
  id: number;
  answer: (yes: boolean) => void;
}

let queue: Asked[] = [];
let next = 1;
const listeners = new Set<() => void>();
const changed = () => {
  for (const fn of [...listeners]) fn();
};
const subscribe = (fn: () => void) => {
  listeners.add(fn);
  return () => listeners.delete(fn);
};
const front = () => queue[0] ?? null;

/** Ask; resolves true when the person confirmed. One at a time, in the order asked. */
export function ask(question: Question): Promise<boolean> {
  return new Promise((resolve) => {
    const asked: Asked = {
      ...question,
      text: question.text.slice(0, 2000),
      id: next++,
      answer: (yes) => {
        queue = queue.filter((q) => q !== asked);
        changed();
        resolve(yes);
      },
    };
    queue = [...queue, asked];
    changed();
  });
}

/** Whether a question is up: every frame is inert meanwhile. */
export function asking(): boolean {
  return queue.length > 0;
}

export function onAsking(fn: () => void): () => void {
  return subscribe(fn);
}

if (typeof window !== "undefined") {
  window.addEventListener(
    "keydown",
    (event) => {
      const q = front();
      if (!q) return;
      const dialog = document.getElementById("hx-ask");
      const inside = event.target instanceof Node && dialog?.contains(event.target);
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        q.answer(false);
      } else if (!inside) {
        // Not the page's while the question is up: back into the question.
        event.preventDefault();
        event.stopImmediatePropagation();
        dialog?.querySelector<HTMLButtonElement>("button")?.focus();
      } else {
        event.stopImmediatePropagation();
      }
    },
    true,
  );
}

/** The question in front, drawn over everything. */
export function QuestionLayer() {
  const t = useT();
  const q = useSyncExternalStore(subscribe, front);
  const cancel = useRef<HTMLButtonElement>(null);
  const before = useRef<Element | null>(null);
  useEffect(() => {
    if (!q) return;
    before.current = document.activeElement;
    cancel.current?.focus();
    return () => {
      if (before.current instanceof HTMLElement && before.current.isConnected) before.current.focus();
    };
  }, [q]);
  if (!q) return null;
  const title = q.title?.trim() || (q.kind === "link" ? t("surface.link.title") : t("surface.confirm.title"));
  return (
    <div className="hx-ask-veil" data-testid="surface-question-veil">
      <div id="hx-ask" role="alertdialog" aria-modal="true" aria-labelledby="hx-ask-title" data-testid="surface-question" data-kind={q.kind}>
        <div className="hx-ask-who" data-testid="surface-question-app">
          {t("surface.confirm.asks", { app: q.app })}
        </div>
        <div className="hx-ask-title" id="hx-ask-title">
          {title}
        </div>
        <div className="hx-ask-text" data-testid="surface-question-text">
          {q.kind === "link" ? <code>{q.text}</code> : q.text}
        </div>
        <div className="hx-ask-foot">
          <button ref={cancel} type="button" className="btn outline" data-testid="surface-question-no" onClick={() => q.answer(false)}>
            {t("common.cancel")}
          </button>
          <button type="button" className={`btn primary${q.danger ? " danger-fill" : ""}`} data-testid="surface-question-yes" onClick={() => q.answer(true)}>
            {q.confirm?.trim() || (q.kind === "link" ? t("surface.link.open") : t("common.confirm"))}
          </button>
        </div>
      </div>
    </div>
  );
}
