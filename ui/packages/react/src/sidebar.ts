/**
 * The right column as a docking surface (harnix-ext.md §22.7): the tab types it opens, how a
 * resource address is claimed by one of them, and what an open tab's body is told.
 *
 * A tab type is a `sidebar.tab` contribution. Two sorts: a *resource* type gives `patterns` and
 * claims `harnix-resource://<type>/…` addresses (a file preview, say); a *page* type gives none and
 * is opened by its `kind` (the guide, a usage page). One `kind` carries at most one `extension`
 * registration and one of the product's (`builtin` or `fallback`); the extension's is the one in
 * force, and the product's comes back when it is withdrawn.
 *
 * Who opens an address follows an editor's resolver: the types whose patterns match, by priority
 * tier (`extension`, `builtin`, `fallback`), then by the length of the pattern that matched, then
 * by registration order; `canOpen` strikes a candidate out. The surface itself — which session's
 * tabs, what is persisted — is the shell's (`@harnix/shell`'s `sidebar/`); this module is the part
 * a UI package needs: the types, the addresses, `useTabInfo()`.
 */

import { createContext, createElement, useContext, type ReactNode } from "react";

/** The tiers a tab type claims an address at, highest first. */
export const SIDEBAR_PRIORITIES = ["extension", "builtin", "fallback"] as const;
export type SidebarPriority = (typeof SIDEBAR_PRIORITIES)[number];

/** `session`: the tab lives in the open session's surface, and is remembered with it; `space`:
 *  the type stands in the strip for the front space while it is in front, as a space's
 *  `details.tabs` do — one tab, not closable. */
export const SIDEBAR_SCOPES = ["session", "space"] as const;
export type SidebarScope = (typeof SIDEBAR_SCOPES)[number];

/** An entry card on the guide page, contributed by a tab type: choosing it opens that type as a page. */
export interface SidebarGuideEntry {
  /** Unique within the type. */
  id: string;
  title(): string;
  description?(): string;
  /** A glyph name, or `<owner>.<name>` for an icon its owner registered. */
  icon?: string;
  order?: number;
}

/** What `openResource` and `openTab` take besides the address or kind. */
export interface SidebarOpenOptions {
  /** Handed to the body as `tab.navigation.params`; a JSON shape, agreed per type (a text preview
   *  takes `{ line?: number }`). */
  params?: Record<string, unknown>;
  /** The same content already open is focused rather than opened twice; false opens beside it. */
  revealIfOpened?: boolean;
  /** The new tab takes the calling tab's place in the strip, and that tab closes (the guide page
   *  hands over to what it opened). */
  replaceTab?: string;
}

export interface SidebarResourceOptions extends SidebarOpenOptions {
  /** Open with this kind's type, skipping the claim (its `canOpen` still has its say). */
  kind?: string;
}

/** A `sidebar.tab` contribution: a tab type the right column can open. */
export interface SidebarTabType {
  /** This implementation's identity among every registration: `<owner>.<kind>` is the natural choice. */
  id: string;
  /** What is opened by name (`openTab(kind)`), and what a tab records. */
  kind: string;
  /** `session` unless said otherwise. */
  scope?: SidebarScope;
  /** Globs over `harnix-resource://` addresses: one with `:` matches the whole address
   *  (`harnix-resource://file/**`); one without matches the address's path at any depth, ignoring
   *  case (`*.md`). None: a page type, opened by kind. */
  patterns?: string[];
  /** `extension` unless said otherwise — the product's own types say `builtin`, a generic viewer
   *  anything specific should beat says `fallback`. */
  priority?: SidebarPriority;
  /** Strike this type out of a claim it matched. */
  canOpen?(address: string): boolean;
  /** The chip's text; the address when the tab shows a resource. Read at every draw. */
  title(address?: string): string;
  /** Its cards on the guide page. */
  guide?: SidebarGuideEntry[];
  /** Every `openTab` opens a tab of its own (a terminal, an output stream) rather than focusing the one there. */
  multiple?: boolean;
  /** Its body stays mounted, hidden, while another tab or another session is in front. */
  keepMounted?: boolean;
  /** A standing tab: open in every session's strip, at its end, with no close — the context of the
   *  session's next call, say. Opened by the shell when the session's surface is drawn; its `guide`
   *  entries are not needed and not shown. */
  pinned?: boolean;
  /** The body: reads `useTabInfo()` for which tab it is, its address, its navigation. */
  render(): ReactNode;
}

/* ── addresses ────────────────────────────────────────────────────────── */

export const RESOURCE_SCHEME = "harnix-resource://";

/** `harnix-resource://<type>/<path>`: the address of a resource of `type` at `path`. */
export function resourceAddress(type: string, path = ""): string {
  const clean = path.replace(/^\/+/, "");
  return `${RESOURCE_SCHEME}${type}/${clean}`;
}

/** The parts of a resource address; null for anything else. */
export function parseResource(address: string): { type: string; path: string } | null {
  if (typeof address !== "string" || !address.startsWith(RESOURCE_SCHEME)) return null;
  const rest = address.slice(RESOURCE_SCHEME.length);
  const slash = rest.indexOf("/");
  const type = slash < 0 ? rest : rest.slice(0, slash);
  if (!type) return null;
  return { type, path: slash < 0 ? "" : rest.slice(slash + 1) };
}

/** A glob as a regular expression: `**` any run (across `/`), `*` any run within a segment, `?` one. */
function globToRegExp(glob: string, flags: string): RegExp {
  let out = "";
  for (let i = 0; i < glob.length; i += 1) {
    const c = glob[i]!;
    if (c === "*") {
      if (glob[i + 1] === "*") {
        i += 1;
        // `**/` also matches nothing: `a/**/b` takes `a/b`.
        if (glob[i + 1] === "/") {
          i += 1;
          out += "(?:.*/)?";
        } else out += ".*";
      } else out += "[^/]*";
    } else if (c === "?") out += "[^/]";
    else out += c.replace(/[.+^${}()|[\]\\]/g, "\\$&");
  }
  return new RegExp(`^${out}$`, flags);
}

/** Whether a pattern claims an address (see `SidebarTabType.patterns`). */
export function patternMatches(pattern: string, address: string): boolean {
  if (pattern.includes(":")) return globToRegExp(pattern, "").test(address);
  const parsed = parseResource(address);
  if (!parsed) return false;
  const re = globToRegExp(pattern, "i");
  // At any depth: the whole path, or any suffix that starts a segment.
  const path = parsed.path;
  if (re.test(path)) return true;
  for (let i = 0; i < path.length; i += 1) if (path[i] === "/" && re.test(path.slice(i + 1))) return true;
  return false;
}

/* ── which type is in force ───────────────────────────────────────────── */

const TIER: Record<SidebarPriority, number> = { extension: 0, builtin: 1, fallback: 2 };

export function priorityOf(type: Pick<SidebarTabType, "priority">): SidebarPriority {
  return type.priority ?? "extension";
}

/** Whether two registrations of one kind collide: both the extension's, or both the product's. */
export function sameTier(a: Pick<SidebarTabType, "priority">, b: Pick<SidebarTabType, "priority">): boolean {
  return (priorityOf(a) === "extension") === (priorityOf(b) === "extension");
}

/** The types in force, one per kind: the extension's over the product's, in registration order. */
export function effectiveTypes<T extends SidebarTabType>(types: readonly T[]): T[] {
  const byKind = new Map<string, T>();
  for (const type of types) {
    const held = byKind.get(type.kind);
    if (!held || (priorityOf(held) !== "extension" && priorityOf(type) === "extension")) byKind.set(type.kind, type);
  }
  return types.filter((type) => byKind.get(type.kind) === type);
}

/** The type in force for a kind, if any. */
export function typeOfKind<T extends SidebarTabType>(types: readonly T[], kind: string): T | null {
  return effectiveTypes(types).find((type) => type.kind === kind) ?? null;
}

/** The types that would open an address, best first; `canOpen` has struck its vetoes out. */
export function candidates<T extends SidebarTabType>(types: readonly T[], address: string): T[] {
  const hits: { type: T; tier: number; length: number; order: number }[] = [];
  effectiveTypes(types).forEach((type, order) => {
    const matched = (type.patterns ?? []).filter((p) => patternMatches(p, address));
    if (!matched.length) return;
    if (type.canOpen && !type.canOpen(address)) return;
    hits.push({ type, tier: TIER[priorityOf(type)], length: Math.max(...matched.map((p) => p.length)), order });
  });
  hits.sort((a, b) => a.tier - b.tier || b.length - a.length || a.order - b.order);
  return hits.map((h) => h.type);
}

/** The type that opens an address: the best candidate, or — with `kind` — that kind's type if it
 *  does not refuse the address. Null when none. */
export function claim<T extends SidebarTabType>(types: readonly T[], address: string, kind?: string): T | null {
  if (kind !== undefined) {
    const type = typeOfKind(types, kind);
    if (!type) return null;
    return type.canOpen && !type.canOpen(address) ? null : type;
  }
  return candidates(types, address)[0] ?? null;
}

/** The guide's entries, from every type in force that has some, by `order` then by type; a
 *  standing type (`pinned`) is in every strip already and offers none. */
export function guideEntries<T extends SidebarTabType>(types: readonly T[]): (SidebarGuideEntry & { kind: string; type: T })[] {
  const out: (SidebarGuideEntry & { kind: string; type: T })[] = [];
  for (const type of effectiveTypes(types)) {
    if (type.pinned === true) continue;
    for (const entry of type.guide ?? []) out.push({ ...entry, kind: type.kind, type });
  }
  return out.sort((a, b) => (a.order ?? 100) - (b.order ?? 100));
}

/* ── the tab domain ───────────────────────────────────────────────────── */

/** What an open tab may do: on its own surface, always. */
export interface TabActions {
  /** Close this tab (its close handlers run first; one that throws keeps it open). */
  close(): void;
  /** Open a resource in this tab's surface; answers the tab's id. */
  openResource(address: string, options?: SidebarResourceOptions): string;
  /** Open a page by kind in this tab's surface; answers the tab's id. */
  openTab(kind: string, options?: SidebarOpenOptions): string;
  /** Run before this tab is closed or replaced; a throw keeps it. Returns the unsubscribe. Not
   *  called when the column collapses, the session changes or the page unloads. */
  onClose(handler: () => void): () => void;
}

/** What `useTabInfo()` answers an open tab's body. */
export interface TabInfo {
  tab: {
    /** The tab's id in its surface, stable while it is open. */
    id: string;
    kind: string;
    /** The resource it shows; null for a page. */
    address: string | null;
    /** The session whose surface it is in: what a page about "this session" reads. Null for a type
     *  standing in the space's side of the strip (`scope: "space"`). */
    session: { instance: string; sessionId: string } | null;
    /** The body is the one in front: not hidden behind another tab, nor kept for a session in the background. */
    visible: boolean;
    /** How it was last navigated to: the params the opener gave, and a count that goes up at each
     *  `openResource`/`openTab` that landed on this tab — including the first. */
    navigation: { params: Record<string, unknown>; revision: number };
    /** Aborted when the tab goes: closed, or its session's surface dropped. */
    signal: AbortSignal;
    actions: TabActions;
  };
}

const TabContext = createContext<TabInfo | null>(null);

/** The shell wraps each tab body in it. */
export function TabInfoProvider({ value, children }: { value: TabInfo; children: ReactNode }) {
  return createElement(TabContext.Provider, { value }, children);
}

/** Which tab this body is, how it was navigated to, and what it may do (harnix-ext.md §22.7). */
export function useTabInfo(): TabInfo {
  const info = useContext(TabContext);
  if (!info) throw new Error("@harnix/react: useTabInfo() is for the body of a sidebar tab");
  return info;
}
