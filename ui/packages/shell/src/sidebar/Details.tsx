/**
 * The right column: one tab strip over one body, with no title bar. The strip has two sides. The
 * front space's — its `details.tabs`, the details a card revealed (`host.reveal`), the
 * `scope: "space"` tab types — is as it was in 0.4.4 and keeps its semantics. The open session's
 * side is its docking surface (`surface.ts`): the tabs opened through the navigation controller
 * (`controller.ts`), each closable, remembered with the session. Which side is in front is the
 * app's `stripShown()`.
 *
 * A session tab's body is drawn inside its tab domain (`useTabInfo()`): which tab it is, how it was
 * navigated to, what it may do. A body of a `keepMounted` type that has been in front stays on the
 * page, hidden, while another tab or another session is — so a terminal's scrollback or a tree's
 * expansion is not lost to a glance elsewhere.
 */

import { useEffect, useMemo, useRef, useSyncExternalStore, type ReactNode } from "react";
import { Contributed, Glyph, TabInfoProvider, sidebarTypeOfKind, useRegistry, useRoving, useT, type SidebarTabType, type TabInfo } from "@harnix/react";

import { keyOf, orderTabs, safeTabs, type App, type SessionKey, type ShellState, type StripTab } from "../app.ts";
import { spacesOf } from "../Sidebar.tsx";
import { GUIDE_KIND, closeTab, openResource, openTab, seedSurface, titleOf } from "./controller.ts";
import type { SidebarTab } from "./surface.ts";

type Type = SidebarTabType & { owner: string };

/** `resizer`: the column's drag edge, which the frame draws (`Shell.tsx`). */
export function Details({ app, s, resizer }: { app: App; s: ShellState; resizer: ReactNode }) {
  const t = useT();
  const registry = useRegistry(app.registry);
  useSyncExternalStore(app.sidebar.subscribe, app.sidebar.getVersion);
  const space = spacesOf(app).find((c) => c.id === s.space);
  const tabs = orderTabs(safeTabs(registry, space));
  const firstEnd = tabs.find((x) => x.align === "end")?.id;
  const item = s.detail;
  const panel = item ? registry.claim("details.panel", item) : null;
  const open = app.detailsOpen(s);
  const current = s.current;
  const types = registry.at("sidebar.tab");
  const sessionTabs = current ? app.sidebar.tabs(current) : [];
  const shown = app.stripShown(s);
  const shownId = shown.side === "space" ? shown.id : shown.tab.id;
  const spaceShown = shown.side === "space" ? tabs.find((x) => x.id === shown.id) : undefined;
  // A session whose surface has nothing yet, in a space whose side has nothing either: the default
  // page (the guide, or the one page there is to open) opens once the column is open.
  const empty = !tabs.length && !item;
  useEffect(() => {
    if (open && current && empty && !sessionTabs.length) seedSurface(app, current);
  }, [app, open, current, empty, sessionTabs.length]);
  const guideOpen = sessionTabs.some((x) => x.kind === GUIDE_KIND);
  const canGuide = current !== null && sidebarTypeOfKind(types, GUIDE_KIND) !== null;
  // One Tab stop for the strip; the arrows go along it.
  const strip = useRef<HTMLDivElement>(null);
  const roving = useRoving(strip, { items: '[role="tab"]', orientation: "horizontal", activate: true });
  const chipTitle = (tab: SidebarTab) => {
    const type = sidebarTypeOfKind(types, tab.kind);
    return type ? titleOf(app, type, tab.address) : tab.title;
  };
  const close = (tab: SidebarTab) => {
    if (current) closeTab(app, tab.id, current);
  };
  return (
    // Closed it is zero wide, not gone: inert, or Tab would still walk through what is in it.
    <aside id="details" aria-hidden={!open} inert={!open}>
      {resizer}
      <div className="strip" data-tauri-drag-region="deep">
        <div className="tabs" role="tablist" ref={strip} onKeyDown={roving.onKeyDown} onFocus={roving.onFocus}>
          {tabs.map((x) => (
            <button
              type="button"
              role="tab"
              key={x.id}
              id={`details-tab-${x.id}`}
              aria-controls="details-panel"
              data-tab={x.id}
              data-scope="space"
              data-align={x.id === firstEnd ? "end" : undefined}
              aria-selected={shownId === x.id}
              className={shownId === x.id ? "on" : ""}
              onClick={() => app.selectTab(x.id)}
            >
              {x.title()}
            </button>
          ))}
          {item || (!tabs.length && !sessionTabs.length) ? (
            <button
              type="button"
              role="tab"
              id="details-tab-details"
              aria-controls="details-panel"
              data-tab="details"
              data-scope="space"
              aria-selected={shownId === "details"}
              className={shownId === "details" ? "on" : ""}
              onClick={() => app.selectTab("details")}
            >
              {item && !tabs.length ? (panel?.title?.(item) ?? item.name ?? t("details.title")) : t("details.title")}
            </button>
          ) : null}
          {sessionTabs.map((tab) => {
            const title = chipTitle(tab);
            const on = shown.side === "session" && shown.tab.id === tab.id;
            return (
              <div key={tab.id} className={`chip${on ? " on" : ""}`} data-testid="sidebar-tab" data-tab={tab.id} data-kind={tab.kind} data-state={on ? "current" : "open"}>
                <button
                  type="button"
                  role="tab"
                  id={`details-tab-${tab.id}`}
                  aria-controls="details-panel"
                  data-tab={tab.id}
                  data-scope="session"
                  aria-selected={on}
                  className={on ? "on" : ""}
                  onClick={() => app.focusSessionTab(tab.id)}
                  onAuxClick={(e) => {
                    if (e.button === 1) close(tab);
                  }}
                >
                  {title}
                </button>
                <button type="button" className="tab-close" data-testid="sidebar-tab-close" title={t("sidebar.closeTab", { title })} aria-label={t("sidebar.closeTab", { title })} onClick={() => close(tab)}>
                  <Glyph name="close" className="glyph" />
                </button>
              </div>
            );
          })}
        </div>
        <div className="win">
          {canGuide && !guideOpen ? (
            <button type="button" className="icon-btn" data-testid="sidebar-add" title={t("sidebar.addTab")} aria-label={t("sidebar.addTab")} onClick={() => openTab(app, GUIDE_KIND)}>
              <Glyph name="plus" className="glyph" />
            </button>
          ) : null}
          <button
            type="button"
            className="icon-btn"
            data-testid="details-max"
            title={s.detailsMax ? t("details.restore") : t("details.maximize")}
            aria-pressed={s.detailsMax}
            onClick={() => app.toggleDetailsMax()}
          >
            <Glyph name={s.detailsMax ? "restore" : "maximize"} className="glyph" />
          </button>
          <button type="button" className="icon-btn close" data-testid="details-close" title={t("common.close")} onClick={() => app.toggleDetails(false)}>
            <Glyph name="close" className="glyph" />
          </button>
        </div>
      </div>
      <div className="body" role="tabpanel" id="details-panel" aria-labelledby={`details-tab-${shownId}`}>
        {shown.side === "space" ? (
          spaceShown && space ? (
            <SpaceTabBody app={app} space={space} tab={spaceShown} />
          ) : !item ? (
            <div className="empty">{t("details.empty")}</div>
          ) : panel ? (
            <Contributed registry={registry} point="details.panel" contribution={panel}>
              {panel.render(item)}
            </Contributed>
          ) : (
            <div className="empty">{t("details.noPanel")}</div>
          )
        ) : null}
        <SessionBodies app={app} current={current} shown={shown.side === "session" && open ? shown.tab.id : null} />
      </div>
    </aside>
  );
}

/** A tab of the space's side: the space's own, or a `scope: "space"` type drawn in a tab domain of its own. */
function SpaceTabBody({ app, space, tab }: { app: App; space: { id: string; owner: string }; tab: StripTab }) {
  const registry = useRegistry(app.registry);
  if (!tab.type) {
    return (
      <Contributed registry={registry} point="space" contribution={space}>
        {tab.render()}
      </Contributed>
    );
  }
  const type = tab.type;
  const info: TabInfo = {
    tab: {
      id: type.kind,
      kind: type.kind,
      address: null,
      session: null,
      visible: true,
      navigation: { params: {}, revision: 1 },
      signal: new AbortController().signal,
      actions: {
        close: () => undefined,
        openResource: (address, options) => openResource(app, address, options),
        openTab: (kind, options) => openTab(app, kind, options),
        onClose: () => () => undefined,
      },
    },
  };
  return (
    <Contributed registry={registry} point="sidebar.tab" contribution={type}>
      <TabInfoProvider value={info}>
        <Body type={type} />
      </TabInfoProvider>
    </Contributed>
  );
}

/** A type's body, called inside the guard: a `render()` that throws costs this tab, not the column. */
function Body({ type }: { type: Type }) {
  return <>{type.render()}</>;
}

/**
 * The session tabs' bodies on the page: the one in front, and every `keepMounted` body that has been
 * in front once — of the open session and of the sessions whose surfaces this page has loaded —
 * hidden. The rest are not mounted until they are in front.
 */
function SessionBodies({ app, current, shown }: { app: App; current: SessionKey | null; shown: string | null }) {
  const registry = useRegistry(app.registry);
  useSyncExternalStore(app.sidebar.subscribe, app.sidebar.getVersion);
  const types = registry.at("sidebar.tab");
  const currentKey = current ? keyOf(current.instance, current.sessionId) : null;
  const bodies: ReactNode[] = [];
  for (const key of app.sidebar.loaded()) {
    const [instance, ...rest] = key.split("/");
    const session = { instance: instance!, sessionId: rest.join("/") };
    for (const tab of app.sidebar.tabs(session)) {
      const type = sidebarTypeOfKind(types, tab.kind);
      const inFront = key === currentKey && tab.id === shown;
      if (!inFront && !(type?.keepMounted && app.sidebar.visited(session, tab.id))) continue;
      bodies.push(<TabBody key={`${key}/${tab.id}`} app={app} session={session} tab={tab} type={type} visible={inFront} />);
    }
  }
  return <>{bodies}</>;
}

function TabBody({ app, session, tab, type, visible }: { app: App; session: SessionKey; tab: SidebarTab; type: Type | null; visible: boolean }) {
  const t = useT();
  const registry = useRegistry(app.registry);
  const navigation = app.sidebar.navigation(session, tab.id);
  useEffect(() => {
    if (visible) app.sidebar.visit(session, tab.id);
  }, [app, session, tab.id, visible]);
  const info = useMemo<TabInfo>(
    () => ({
      tab: {
        id: tab.id,
        kind: tab.kind,
        address: tab.address,
        session,
        visible,
        navigation,
        signal: app.sidebar.signal(session, tab.id),
        actions: {
          close: () => void closeTab(app, tab.id, session),
          openResource: (address, options) => openResource(app, address, options, session),
          openTab: (kind, options) => openTab(app, kind, options, session),
          onClose: (handler) => app.sidebar.onClose(session, tab.id, handler),
        },
      },
    }),
    // A navigation is a new object each time the tab is navigated to: the body sees it change.
    [app, session, tab.id, tab.kind, tab.address, visible, navigation],
  );
  return (
    <div className="tab-body" data-testid="sidebar-tab-body" data-tab={tab.id} data-kind={tab.kind} hidden={!visible}>
      {type ? (
        <Contributed registry={registry} point="sidebar.tab" contribution={type}>
          <TabInfoProvider value={info}>
            <Body type={type} />
          </TabInfoProvider>
        </Contributed>
      ) : (
        <div className="empty">{t("sidebar.noType", { kind: tab.kind })}</div>
      )}
    </div>
  );
}
