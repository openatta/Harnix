/**
 * The shell's own question to the person, on an application's behalf: `confirm` and `openLink`
 * over the bridge. Drawn by the page, over everything — every frame, and the shell's own dialog
 * if one is open (a settings section's frame may ask) — so no application can draw it, cover it
 * or answer it: its frames are inert while it is up, and it names who is asking.
 *
 * It is a layer (`../layers.ts`) of the top rank: over every dialog whenever it came, the focus in
 * it and back where it was after, and every key its while it is open — Escape answers no, and
 * nothing else on the page hears any (ipdrun-38). Not by being the first listener on the page: the
 * layers' one listener asks which layer is on top.
 */

import { useRef, useSyncExternalStore } from "react";
import { useT } from "@harnix/react";

import { useLayer } from "../layers.ts";

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

/** The question in front, drawn over everything. */
export function QuestionLayer() {
  const q = useSyncExternalStore(subscribe, front);
  return q ? <QuestionBox key={q.id} q={q} /> : null;
}

function QuestionBox({ q }: { q: Asked }) {
  const t = useT();
  const box = useRef<HTMLDivElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  useLayer(box, { rank: 1, close: () => q.answer(false), initial: () => cancel.current });
  const title = q.title?.trim() || (q.kind === "link" ? t("surface.link.title") : t("surface.confirm.title"));
  return (
    <div className="hx-ask-veil" data-testid="surface-question-veil">
      <div ref={box} id="hx-ask" role="alertdialog" aria-modal="true" aria-labelledby="hx-ask-title" data-testid="surface-question" data-kind={q.kind}>
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
