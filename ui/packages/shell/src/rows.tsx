/**
 * One session in the left column, as the shell draws it for a space with no column of its own (and
 * for an application's grouping, `inline/Grouping.tsx`): its name, whether a turn runs, how long
 * ago it was active, and its menu — on `⋯`, or a right click.
 */

import { Glyph, ago, useT } from "@harnix/react";

import { keyOf, type App, type ShellSession, type ShellState } from "./app.ts";
import { sessionMenu } from "./dialogs.tsx";
import { menuBelow, menuPoint } from "./Menu.tsx";

export function SessionRow({ app, s, session, nested = false }: { app: App; s: ShellState; session: ShellSession; nested?: boolean }) {
  const t = useT();
  const instance = session.instance;
  const on = s.current?.sessionId === session.session_id && s.current.instance === instance;
  const running = s.running[keyOf(instance, session.session_id)];
  const tooltip = [
    session.name || t("sidebar.untitled"),
    session.scene || session.status || "",
    session.message_count ? t("sidebar.messageCount", { count: session.message_count }) : "",
  ]
    .filter(Boolean)
    .join(" · ");
  const menu = (event: { clientX?: number; clientY?: number; currentTarget: EventTarget | null }) =>
    sessionMenu(app, session, { origin: event.currentTarget instanceof HTMLElement ? event.currentTarget : null, at: menuPoint(event) });
  // `⋯` is the row's sibling, not inside it: a button in a button is neither HTML nor reachable by Tab.
  // Nested under a group it is an item of the group's tree.
  return (
    <div className={`srow-wrap${on ? " on" : ""}${nested ? " nested" : ""}`} role={nested ? "none" : undefined}>
      <button
        type="button"
        className={`srow${on ? " on" : ""}${nested ? " nested" : ""}`}
        {...(nested ? { role: "treeitem", "aria-level": 2, "aria-selected": on } : {})}
        data-testid="session-row"
        data-state={on ? "current" : undefined}
        data-sid={session.session_id}
        data-instance={instance}
        title={tooltip}
        onClick={() => app.open({ instance, sessionId: session.session_id })}
        onContextMenu={(e) => {
          e.preventDefault();
          menu(e);
        }}
      >
        {running ? <span className="live" /> : null}
        <span className="tt name">{session.name || t("sidebar.untitled")}</span>
        <span className="meta time">{ago(session.last_active, t)}</span>
      </button>
      <button
        type="button"
        className="more"
        data-testid="session-more"
        title={t("sidebar.sessionActions")}
        aria-label={t("sidebar.sessionActions")}
        aria-haspopup="menu"
        onClick={(e) => sessionMenu(app, session, { origin: e.currentTarget, at: menuBelow(e.currentTarget) })}
      >
        <Glyph name="more" className="glyph" />
      </button>
    </div>
  );
}
