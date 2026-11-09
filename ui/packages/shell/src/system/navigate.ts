/** Out of the system space, back to a session where it belongs. */

import type { App, SessionKey } from "../app.ts";
import { SYSTEM } from "./constants.ts";
import type { SessionRowView } from "./types.ts";

/** Back from the system space to a session where it belongs (the session space lists by time and
 *  by project, so a session of `sessions` may be shown by another), then `then` once it is open. */
export function openWhereItBelongs(app: App, session: SessionKey, space: string): void {
  const target = app.registry.at("space").find((c) => (c.sessions ?? c.id) === space && c.id !== SYSTEM)?.id ?? space;
  app.open(session, target);
  void app.switchSpace(target);
}

/**
 * From a call back to the conversation: the session opened where it belongs, and the tool call it
 * asked for (if it asked for one) scrolled to and flashed. The row is drawn once the history is in,
 * so it is looked for a few times.
 */
export async function locateInConversation(app: App, session: SessionKey, tool: string | null): Promise<void> {
  const listed = await app.call<{ sessions: SessionRowView[] }>("harnix.sessions", { space: SYSTEM }).catch(() => ({ sessions: [] }));
  const row = listed.sessions.find((r) => r.session_id === session.sessionId && (r.instance ?? "system") === session.instance);
  openWhereItBelongs(app, session, row?.space ?? "sessions");
  if (!tool) return;
  for (let i = 0; i < 25; i += 1) {
    await new Promise((resolve) => setTimeout(resolve, 200));
    const node = document.querySelector<HTMLElement>(`[data-testid="conv-tool"][data-id="${CSS.escape(tool)}"]`);
    if (!node) continue;
    node.scrollIntoView({ block: "center" });
    node.classList.add("hx-flash");
    node.dataset.located = "true";
    setTimeout(() => node.classList.remove("hx-flash"), 2000);
    return;
  }
}
