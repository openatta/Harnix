/**
 * The points the projects space defines for other packages to fill (harnix-app.md §3.2): what
 * else a project holds besides its conversations, and what its overview page shows.
 *
 *   harnix-app.project.items   a kind of item in the project tree — its rows for a project root,
 *                              how a row is drawn and opened, and the entry it adds to the
 *                              project's "+" menu. The flows register their tasks here.
 *   harnix-app.project.panels  a block on a project's overview page.
 *   harnix-app.session.badge   a badge on a session's row in the left column (both spaces'): a
 *                              schedule's clock.
 *
 * The projects space knows no item kind by name: it draws whatever is registered, interleaved
 * with the conversations by activity. A kind's `rows` are read on each draw and `subscribe` says
 * when they changed, so a kind keeps its own cache (a topic's last value) and the tree follows it
 * without knowing where the rows come from.
 */

import { useEffect, useMemo, useReducer, type ReactNode } from "react";
import { useContributions, type MenuItem, type Registry } from "@harnix/react";

import type { Workspace } from "./store.ts";

export const PROJECT_ITEMS = "harnix-app.project.items";
export const PROJECT_PANELS = "harnix-app.project.panels";
export const SESSION_BADGE = "harnix-app.session.badge";

/** One row of an item kind, in a project. */
export interface ProjectItemRow {
  /** Unique within its kind. */
  id: string;
  /** The row's name, for its tooltip and the tree's accessible name. */
  title: string;
  /** When it was last active (milliseconds since the epoch, or an ISO date): the tree orders rows
   *  and conversations together by it, newest first. Null: at the end. */
  at?: number | string | null;
  /** Something is happening in it now: the project shows the live dot while closed. */
  running?: boolean;
  /** The session it opens in the middle, when it has one: the row is marked current while that
   *  session is open. */
  session?: { instance: string; sessionId: string } | null;
  /** Whatever the kind wants to carry to `render`, `open` and `menu`. */
  data?: unknown;
}

export interface ProjectItemContext {
  /** The project's directory. */
  root: string;
  workspace: Workspace;
  /** The backend space the projects space lists sessions of: where a kind shows a session it
   *  owns so that it opens here (`harnix.session.expose`). */
  space: string;
}

/** A kind of item in the project tree (`harnix-app.project.items`, a `register` point). */
export interface ProjectItemKind {
  /** `<owner>.<kind>`: the flows' is `flow.task`. */
  id: string;
  /** The kind's name, plural, as a heading or a menu says it ("任务"). */
  title(): string;
  /** Its rows for a project root; read on each draw, so answer from a cache. */
  rows(root: string): ProjectItemRow[];
  /** Tell the tree the rows changed. */
  subscribe(fn: () => void): () => void;
  /** The row's body, inside the tree's button (the name, a status, a badge). */
  render(row: ProjectItemRow, context: ProjectItemContext): ReactNode;
  /** The row was clicked. */
  open(row: ProjectItemRow, context: ProjectItemContext): void | Promise<void>;
  /** The row's menu (right click, its `⋯`); none: the row has no menu. */
  menu?(row: ProjectItemRow, context: ProjectItemContext): MenuItem[];
  /** The entry in the project's "+" menu (and its own menu) that makes one: "新任务". */
  create?: { label(): string; icon?: string; run(context: ProjectItemContext): void | Promise<void> };
}

/** A block on a project's overview page (`harnix-app.project.panels`, a `render` point). */
export interface ProjectPanel {
  id: string;
  title(): string;
  /** Smaller first; 100 when absent. */
  order?: number;
  render(context: ProjectItemContext): ReactNode;
}

/** A badge on a session's row in the left column (`harnix-app.session.badge`, a `render` point):
 *  another package's word about the session — a schedule's clock, say. */
export interface SessionBadge {
  id: string;
  /** Inside the row, before its time; nothing to say, nothing drawn (null). Read at every draw of the row. */
  render(session: { instance: string; sessionId: string }): ReactNode;
}

/** A kind's row, with the kind that owns it. */
export interface ProjectItem {
  kind: ProjectItemKind & { owner: string };
  row: ProjectItemRow;
}

/** When a row was last active, in milliseconds; null when it says nothing. */
export function activityOf(at: number | string | null | undefined): number | null {
  if (typeof at === "number" && Number.isFinite(at)) return at;
  if (typeof at === "string" && at) {
    const n = Date.parse(at);
    return Number.isNaN(n) ? null : n;
  }
  return null;
}

/**
 * The registered kinds' rows for a project, kept current: the kinds from the registry, each
 * followed by its `subscribe`, and the rows read again whenever one says they changed. A kind
 * whose `rows` or `subscribe` throws is counted as failed and contributes nothing.
 */
export function useProjectItems(r: Registry, root: string | null): ProjectItem[] {
  const kinds = useContributions<ProjectItemKind>(PROJECT_ITEMS, r);
  const [version, changed] = useReducer((n: number) => n + 1, 0);
  const ids = kinds.map((k) => k.id).join("\n");
  useEffect(() => {
    if (!root) return undefined;
    const offs = kinds.map((kind) => {
      try {
        return kind.subscribe(changed);
      } catch (e) {
        r.fail(PROJECT_ITEMS, kind, e);
        return () => undefined;
      }
    });
    changed();
    return () => {
      for (const off of offs) off();
    };
  }, [r, ids, root]);
  return useMemo(() => {
    if (!root) return [];
    return kinds.flatMap((kind) => {
      try {
        return kind.rows(root).map((row) => ({ kind, row }));
      } catch (e) {
        // Counted once the draw is over: the registry's listeners are not told from inside a render.
        queueMicrotask(() => r.fail(PROJECT_ITEMS, kind, e));
        return [];
      }
    });
  }, [r, ids, root, version]);
}

/** The kinds that offer to make one, as entries of the project's "+" menu. */
export function createEntries(r: Registry, context: ProjectItemContext): MenuItem[] {
  return r.list<ProjectItemKind>(PROJECT_ITEMS).flatMap((kind) => {
    if (!kind.create) return [];
    try {
      return [{ id: `item:${kind.id}`, icon: kind.create.icon, label: kind.create.label(), run: () => kind.create!.run(context) }];
    } catch (e) {
      r.fail(PROJECT_ITEMS, kind, e);
      return [];
    }
  });
}
