/**
 * HOME (design §8.6): a dashboard in Helm's shape — a serif greeting and the date, an ask box that
 * opens HOME's own conversation, and the `home.card` registrations as a grid.
 */

import { useEffect, useState, type ReactNode } from "react";
import { Contributed, currentLocale, useRegistry, useT } from "@harnix/react";

import type { App, ShellSession } from "./app.ts";
import { Glyph } from "./glyph.tsx";
import { greeting } from "./Shell.tsx";

export function HomeOverview({ app, ask }: { app: App; ask: ReactNode }) {
  const t = useT();
  const registry = useRegistry(app.registry);
  const cards = [...registry.at("home.card")].sort((a, b) => (a.order ?? 100) - (b.order ?? 100));
  const today = new Date().toLocaleDateString(currentLocale(), { month: "long", day: "numeric", weekday: "short" });
  return (
    <div className="home">
      <div className="dash-hd">
        <h2 className="home-title">{t("home.greeting", { greeting: greeting(t) })}</h2>
        <span className="date">{today}</span>
      </div>
      {ask}
      {cards.length ? (
        <div className="dgrid home-cards">
          {cards.map((card) => (
            <section key={card.id} className="dcard home-card" data-card={card.id}>
              {card.title ? <div className="dt">{card.title()}</div> : null}
              <Contributed registry={registry} point="home.card" contribution={card}>
                {card.render()}
              </Contributed>
            </section>
          ))}
        </div>
      ) : (
        <div className="empty">{t("home.empty")}</div>
      )}
    </div>
  );
}

/** The built-in card: the session space's latest conversations, one click from being open there. */
export function RecentSessions({ app }: { app: App }) {
  const t = useT();
  const [rows, setRows] = useState<ShellSession[] | null>(null);
  useEffect(() => {
    let alive = true;
    app
      .call<{ sessions: ShellSession[] }>("harnix.sessions", { space: "sessions" })
      .then((r) => {
        if (!alive) return;
        const sorted = [...r.sessions].sort((a, b) => String(b.last_active ?? "").localeCompare(String(a.last_active ?? "")));
        setRows(sorted.slice(0, 5));
      })
      .catch(() => alive && setRows([]));
    return () => {
      alive = false;
    };
  }, [app]);
  if (!rows) return <div className="muted">{t("common.loading")}</div>;
  if (!rows.length) return <div className="muted">{t("home.recent.empty")}</div>;
  return (
    <div className="list">
      {rows.map((row) => (
        <button
          type="button"
          key={row.session_id}
          className="li"
          data-sid={row.session_id}
          onClick={async () => {
            await app.switchSpace("sessions");
            app.open({ instance: row.instance ?? "system", sessionId: row.session_id }, "sessions");
          }}
        >
          <Glyph name="chat" className="glyph" />
          <span className="tx truncate">{row.name || t("sidebar.untitled")}</span>
        </button>
      ))}
    </div>
  );
}
