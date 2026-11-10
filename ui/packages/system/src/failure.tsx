/**
 * What the system space says when a call fails. The backend's message is for whoever reads its
 * logs (``session `Djxi…` is running turn `t-b53…` ``); the page says what it means for the person
 * looking — "this session is running a turn; this shows once it is over" — and keeps the backend's
 * own words, with the code, in the title. A code it does not know is said in the backend's words.
 */

import { errorText } from "@harnix/core";
import { t, useT } from "@harnix/react";

/** The codes the system space has words for: Harnix's own (`-31xxx`), Agix's (`-32000…-32099`),
 *  and JSON-RPC's "no such method". */
const SAID: Record<number, string> = {
  [-31000]: "system.error.refused",
  [-31002]: "system.error.noInstance",
  [-31003]: "system.error.space",
  [-31004]: "system.error.unavailable",
  [-31005]: "system.error.tooMany",
  [-32000]: "system.error.noSession",
  [-32015]: "system.error.busy",
  [-32018]: "system.error.noTask",
  [-32601]: "system.error.noMethod",
};

function codeOf(e: unknown): number | undefined {
  const code = e && typeof e === "object" ? (e as { code?: unknown }).code : undefined;
  return typeof code === "number" ? code : undefined;
}

/** The sentence for `e`: the page's own for a code it knows, else the backend's message. */
export function failureText(e: unknown): string {
  const key = SAID[codeOf(e) ?? 0];
  return key ? t(key) : errorText(e);
}

/** The backend's own words for `e`, and its code: what the title keeps. */
export function failureDetail(e: unknown): string {
  const code = codeOf(e);
  return code === undefined ? errorText(e) : `${errorText(e)} (${code})`;
}

/** A pane's failure, said as `failureText` with the backend's words in the title. */
export function Failure({ error, testId, className = "empty" }: { error: unknown; testId?: string; className?: string }) {
  useT();
  return (
    <div className={className} data-testid={testId} data-code={codeOf(error)} title={failureDetail(error)}>
      {failureText(error)}
    </div>
  );
}
