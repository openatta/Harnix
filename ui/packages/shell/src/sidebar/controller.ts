/**
 * The right column's navigation controller (harnix-ext.md §22.7): every way into the column —
 * `host.sidebar.*`, a tab's own `actions`, the guide's cards — goes through here, so that one place
 * decides which type opens what, deduplicates, expands the column and focuses what it opened.
 *
 * An address no type claims, or a kind no type has, is a wiring error and throws; so does opening
 * a session tab while no session is open. A type of `scope: "space"` has no tab of its own to
 * open: `openTab(kind)` selects it in the strip, as `host.details.open(kind)` would.
 */

import { sidebarClaim, sidebarGuideEntries, sidebarTypeOfKind, sidebarTypes, t, type SidebarOpenOptions, type SidebarResourceOptions, type SidebarTabType } from "@harnix/react";

import type { App, SessionKey } from "../app.ts";

/** The guide's kind: what an empty surface is seeded with when there is not exactly one entry. */
export const GUIDE_KIND = "guide";

/** The types in force for a kind are read from the registry at each call: an extension may have come or gone. */
function types(app: App): (SidebarTabType & { owner: string })[] {
  return app.registry.at("sidebar.tab");
}

/** The kinds whose type stands in every session's strip (`pinned`), among `all`. */
export function pinnedKinds(all: (SidebarTabType & { owner: string })[]): Set<string> {
  return new Set(
    sidebarTypes(all)
      .filter((type) => type.pinned === true && type.scope !== "space")
      .map((type) => type.kind),
  );
}

/** A type's title for an address; one that throws costs it the title, not the tab. */
export function titleOf(app: App, type: SidebarTabType & { owner: string }, address: string | null): string {
  try {
    return type.title(address ?? undefined);
  } catch (e) {
    app.registry.fail("sidebar.tab", type, e);
    return type.kind;
  }
}

/** The session a tab is opened in: the one given, else the front space's open one. */
function sessionFor(app: App, session: SessionKey | undefined): SessionKey {
  const target = session ?? app.snapshot.current;
  if (!target) throw new Error(t("sidebar.noSession"));
  return target;
}

/** Open a page of `type` in a session's surface, or select it in the strip when it stands for the space. */
function openType(app: App, type: SidebarTabType & { owner: string }, address: string | null, options: SidebarOpenOptions, session: SessionKey | undefined): string {
  if (type.scope === "space") {
    app.toggleDetails(true);
    app.selectTab(type.kind, true);
    return type.kind;
  }
  const target = sessionFor(app, session);
  const id = app.sidebar.open(target, {
    kind: type.kind,
    address,
    title: titleOf(app, type, address),
    params: options.params,
    multiple: type.multiple === true,
    revealIfOpened: options.revealIfOpened,
    replaceTab: options.replaceTab,
  });
  // What is opened is meant to be seen: the column expands, and the session's side of the strip
  // comes in front — unless the tab is a background session's, whose surface is not drawn now.
  if (app.snapshot.current && app.snapshot.current.instance === target.instance && app.snapshot.current.sessionId === target.sessionId) {
    app.toggleDetails(true);
    app.focusSessionTab(id);
  }
  return id;
}

/** `host.sidebar.openResource`: the type that claims the address (or `kind`'s) opens it. */
export function openResource(app: App, address: string, options: SidebarResourceOptions = {}, session?: SessionKey): string {
  if (typeof address !== "string" || !address) throw new Error(t("sidebar.unclaimed", { address: String(address) }));
  const type = sidebarClaim(types(app), address, options.kind);
  if (!type) throw new Error(options.kind !== undefined ? t("sidebar.noKind", { kind: options.kind }) : t("sidebar.unclaimed", { address }));
  return openType(app, type, address, options, session);
}

/** `host.sidebar.openTab`: the page type of `kind` opens. */
export function openTab(app: App, kind: string, options: SidebarOpenOptions = {}, session?: SessionKey): string {
  const type = sidebarTypeOfKind(types(app), kind);
  if (!type) throw new Error(t("sidebar.noKind", { kind }));
  return openType(app, type, null, options, session);
}

/** `host.sidebar.close`: nothing for a tab that is not there, whose close handler keeps it, or that stands (`pinned`). */
export function closeTab(app: App, tabId: string, session?: SessionKey): boolean {
  const target = session ?? app.snapshot.current;
  if (!target) return false;
  const tab = app.sidebar.tab(target, tabId);
  if (tab && pinnedKinds(types(app)).has(tab.kind)) return false;
  return app.sidebar.close(target, tabId);
}

/** `host.sidebar.active`: the open session's tab in front. */
export function activeTab(app: App): { id: string; kind: string; address: string | null } | null {
  const current = app.snapshot.current;
  const tab = current ? app.sidebar.active(current) : null;
  return tab ? { id: tab.id, kind: tab.kind, address: tab.address } : null;
}

/**
 * The default page of a surface that has nothing of its own in it yet (the standing tabs aside):
 * with exactly one guide entry among the types in force, that entry's page; otherwise the guide
 * itself, when a guide type is registered. Nothing when there is no session, or the surface
 * already has a tab of its own.
 */
export function seedSurface(app: App, session: SessionKey): void {
  const all = types(app);
  const standing = pinnedKinds(all);
  if (app.sidebar.tabs(session).some((tab) => !standing.has(tab.kind))) return;
  const entries = sidebarGuideEntries(all);
  const kind = entries.length === 1 ? entries[0]!.kind : sidebarTypeOfKind(all, GUIDE_KIND) ? GUIDE_KIND : null;
  if (!kind) return;
  const type = sidebarTypeOfKind(all, kind);
  if (!type || type.scope === "space") return;
  app.sidebar.open(session, { kind, address: null, title: titleOf(app, type, null), multiple: type.multiple === true });
}
