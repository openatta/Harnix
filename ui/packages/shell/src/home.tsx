/**
 * HOME: an overview of `home.card` registrations — usually one summary card per space — with its
 * own general session as a bar under it (design §8.6).
 */

import { useEffect, useState } from "react";
import { Contributed, useRegistry, useT } from "@harnix/react";

import type { App, ShellSession } from "./app.ts";
import { Glyph } from "./glyph.tsx";

export function HomeOverview({ app }: { app: App }) {
  const t = useT();
  const registry = useRegistry(app.registry);
  const cards = [...registry.at("home.card")].sort((a, b) => (a.order ?? 100) - (b.order ?? 100));
  return (
    <div className="home">
      <h1 className="home-title">{t("home.title")}</h1>
      {cards.length ? (
        <div className="home-cards">
          {cards.map((card) => (
            <section key={card.id} className="home-card" data-card={card.id}>
              {card.title ? <h3>{card.title()}</h3> : null}
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
  if (!rows) return <div className="hint">{t("common.loading")}</div>;
  if (!rows.length) return <div className="hint">{t("home.recent.empty")}</div>;
  return (
    <div className="menu-list">
      {rows.map((row) => (
        <button
          type="button"
          key={row.session_id}
          data-sid={row.session_id}
          onClick={async () => {
            await app.switchSpace("sessions");
            app.open({ instance: row.instance ?? "system", sessionId: row.session_id }, "sessions");
          }}
        >
          <Glyph name="chat" className="glyph" />
          <span className="truncate">{row.name || t("sidebar.untitled")}</span>
        </button>
      ))}
    </div>
  );
}
