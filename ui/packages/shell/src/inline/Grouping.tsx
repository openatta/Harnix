/**
 * `sidebar.group`, declared: how one of an application's spaces groups its sessions in the left
 * column, as its manifest's `[[inline.sidebar_group]]` says — by scene, by a field of the session's
 * row (an array puts it in a group per element), or as the application's own method answers
 * (`{space, sessions}` → `{groups: [{key, title, sessions: [id]}]}`). What no rule places is in the
 * `other` group, last.
 *
 * Only the application's own space: what it declares never regroups the product's lists. A method
 * that fails leaves the list ungrouped, and is noted in the diagnostics.
 */

import { useEffect, useMemo, useState } from "react";
import { errorText } from "@harnix/core";
import { t, useT } from "@harnix/react";

import { keyOf, type App, type ShellSession, type ShellState } from "../app.ts";
import { SessionRow } from "../sessions.tsx";
import { appsOf, type GroupDecl, type RuntimeApp } from "./apps.ts";
import { localized, lookup, words, type Text } from "./template.ts";

export interface Rule {
  owner: RuntimeApp;
  decl: GroupDecl;
}

/** The grouping a running application declares for `space`, if any. */
export function groupingOf(app: App, space: string): Rule | null {
  for (const owner of appsOf(app).running()) {
    const decl = (owner.inline.sidebar_group ?? []).find((g) => (g.space ?? owner.id) === space);
    if (decl) return { owner, decl };
  }
  return null;
}

interface Group {
  key: string;
  title: string;
  rows: ShellSession[];
}

const OTHER = "other";

/** Groups by a value of each row: the values in order, then the rows that have none. */
function byValue(rows: ShellSession[], value: (row: ShellSession) => unknown, other: string): Group[] {
  const groups = new Map<string, Group>();
  const rest: ShellSession[] = [];
  for (const row of rows) {
    const raw = value(row);
    const values = (Array.isArray(raw) ? raw : [raw]).map(words).filter(Boolean);
    if (!values.length) rest.push(row);
    for (const v of values) {
      const group = groups.get(v) ?? { key: `v:${v}`, title: v, rows: [] };
      group.rows.push(row);
      groups.set(v, group);
    }
  }
  const out = [...groups.values()].sort((a, b) => a.title.localeCompare(b.title));
  if (rest.length) out.push({ key: OTHER, title: other, rows: rest });
  return out;
}

/** What the application's method answered, as groups over these rows. */
function fromAnswer(rows: ShellSession[], answer: unknown, other: string): Group[] {
  const listed = (answer as { groups?: { key?: unknown; title?: Text; sessions?: unknown[] }[] } | null)?.groups;
  const byId = new Map(rows.map((r) => [r.session_id, r]));
  const placed = new Set<string>();
  const out: Group[] = [];
  for (const [i, g] of (Array.isArray(listed) ? listed : []).slice(0, 50).entries()) {
    const ids = (Array.isArray(g?.sessions) ? g.sessions : []).filter((id): id is string => typeof id === "string" && byId.has(id));
    if (!ids.length) continue;
    ids.forEach((id) => placed.add(id));
    const key = typeof g.key === "string" && g.key ? g.key : String(i);
    out.push({ key: `m:${key}`, title: localized(g.title) || key, rows: ids.map((id) => byId.get(id)!) });
  }
  const rest = rows.filter((r) => !placed.has(r.session_id));
  if (rest.length) out.push({ key: OTHER, title: other, rows: rest });
  return out;
}

export function GroupedSessions({ app, s, rule, rows }: { app: App; s: ShellState; rule: Rule; rows: ShellSession[] }) {
  const tr = useT();
  const { owner, decl } = rule;
  const other = localized(decl.other) || tr("apps.group.other");
  const ids = rows.map((r) => `${r.instance}/${r.session_id}`).join(",");
  const [answer, setAnswer] = useState<{ ids: string; value: unknown } | null>(null);
  const space = decl.space ?? owner.id;
  useEffect(() => {
    if (decl.by !== "method" || !decl.method) return;
    let alive = true;
    const sessions = rows.map((r) => ({ session_id: r.session_id, instance: r.instance, name: r.name ?? null, scene: r.scene ?? null, status: r.status ?? null, last_active: r.last_active ?? null }));
    app
      .call(`app.${owner.id}.${decl.method}`, { space, sessions })
      .then((value) => alive && setAnswer({ ids, value }))
      .catch((e: unknown) => {
        if (!alive) return;
        app.registry.note({ point: "sidebar.group", owner: owner.id, id: `inline.sidebar_group.${space}`, reason: errorText(e) });
        setAnswer({ ids, value: null });
      });
    return () => {
      alive = false;
    };
  }, [app, owner.id, decl.by, decl.method, space, ids]);
  const groups = useMemo((): Group[] | null => {
    if (decl.by === "scene") return byValue(rows, (r) => r.scene, other);
    if (decl.by === "field") return byValue(rows, (r) => lookup(r as unknown as Record<string, unknown>, decl.field ?? ""), other);
    if (!answer || answer.ids !== ids) return null;
    return answer.value == null ? [{ key: OTHER, title: other, rows }] : fromAnswer(rows, answer.value, other);
  }, [decl.by, decl.field, rows, answer, ids, other]);
  if (!groups) return null;
  return (
    <>
      {groups.map((group) => (
        <div key={group.key} className="group" data-group={group.key} data-testid="space-group" data-app={owner.id}>
          <div className="grp title">
            <span className="label truncate" title={t("apps.card.from", { name: owner.title })}>
              {group.title}
            </span>
            <span className="count">{group.rows.length}</span>
          </div>
          {group.rows.map((row) => (
            <SessionRow key={keyOf(row.instance ?? "system", row.session_id)} app={app} s={s} session={row} />
          ))}
        </div>
      ))}
    </>
  );
}
