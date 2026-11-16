/**
 * The guide: the right column's default page. One card per `guide` entry of the tab types in force;
 * choosing one opens that type's page in the guide's place, so the guide hands over to what it
 * opened. Nothing registered yet: it says so, rather than standing empty.
 */

import { Glyph, sidebarGuideEntries, useRegistry, useT, useTabInfo } from "@harnix/react";

import type { App } from "../app.ts";

/** How many entries may carry their description; a longer list is names alone. */
const DESCRIBED_UP_TO = 4;

export function GuideTab({ app }: { app: App }) {
  const t = useT();
  const registry = useRegistry(app.registry);
  const { tab } = useTabInfo();
  const entries = sidebarGuideEntries(registry.at("sidebar.tab"));
  return (
    <div className="guide" data-testid="sidebar-guide">
      <div className="note">{t("sidebar.guideHint")}</div>
      {entries.length ? (
        <div className="guide-cards">
          {entries.map((entry) => (
            <button
              type="button"
              key={`${entry.kind}:${entry.id}`}
              className="guide-card"
              data-testid="sidebar-guide-entry"
              data-kind={entry.kind}
              data-entry={entry.id}
              onClick={() => tab.actions.openTab(entry.kind, { replaceTab: tab.id })}
            >
              <Glyph name={entry.icon ?? "panel"} className="glyph" />
              <span className="text">
                <span className="title">{entry.title()}</span>
                {entries.length <= DESCRIBED_UP_TO && entry.description ? <span className="desc">{entry.description()}</span> : null}
              </span>
            </button>
          ))}
        </div>
      ) : (
        <div className="empty">{t("sidebar.guideEmpty")}</div>
      )}
    </div>
  );
}
