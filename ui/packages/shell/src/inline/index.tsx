/**
 * The inline points runtime applications declare (`[inline]` in a manifest), drawn by the shell
 * (UI design §4): no code of the application's runs in the page for any of them. What the shell
 * registers for an application goes through the same points a module's registrations do, signed
 * with the application's id, so the diagnostics name it and a later registration still wins:
 *
 * - `tool.row` — its tools' calls, with the row its manifest describes (`ToolRow.tsx`);
 * - `flow.block` — the entries it appends to a session whose event is a declared kind, as cards
 *   (`Card.tsx`);
 * - `command` — its `/` commands and `@` sources, whose candidates its backend's
 *   `app.<id>.complete` answers; a `/` candidate may name one of its methods, which choosing it
 *   runs. The menus are the shell's, as they are for every command;
 * - `sidebar.group` — not a registration: one of its spaces' left column, grouped as declared
 *   (`Grouping.tsx`). The product's own lists keep their grouping.
 *
 * Followed from `harnix.apps`: an application that starts has its points drawn, one that stops or
 * is updated has them taken back (and registered afresh), without the page being reloaded.
 */

import type { Block, BlockOf } from "@harnix/core/state";
import { useSyncExternalStore } from "react";
import { t, type CommandCandidate, type CommandContext, type Contributions, type Point } from "@harnix/react";

import type { App } from "../app.ts";
import { topicsOf } from "../topics.ts";
import { appsOf, toolNames, type CommandDecl, type RuntimeApp } from "./apps.ts";
import { InlineCard, type Entry } from "./Card.tsx";
import { localized, words, type Text } from "./template.ts";
import { InlineToolRow, inlineTitle } from "./ToolRow.tsx";

export { GroupedSessions, groupingOf } from "./Grouping.tsx";
export { appsOf, type Inline } from "./apps.ts";

const APPS_TOPIC = "harnix.apps";
const APPS_LIST = "harnix.apps.list";

/** Start following the applications, once this page can (`harnix.apps.list` is reachable). */
export function startInline(app: App): void {
  let started = false;
  const start = () => {
    if (started || !app.snapshot.reachable.includes(APPS_LIST)) return;
    started = true;
    off();
    const watch = appsOf(app);
    const made = new Map<string, { key: string; ids: [Point, string][] }>();
    topicsOf(app.client).subscribe(APPS_TOPIC, {}, (value) => {
      watch.take(value);
      const running = new Map(watch.running().map((a) => [a.id, a]));
      for (const [id, was] of made) {
        const now = running.get(id);
        if (now && keyOf(now) === was.key) continue;
        for (const [point, cid] of was.ids) app.registry.unregister(point, cid, id);
        made.delete(id);
      }
      for (const a of running.values()) {
        if (!made.has(a.id)) made.set(a.id, { key: keyOf(a), ids: register(app, a) });
      }
    });
  };
  const off = app.subscribe(start);
  start();
}

function keyOf(a: RuntimeApp): string {
  return JSON.stringify([a.title, a.inline]);
}

/** Register what `a` declares; answers what was registered. */
function register(app: App, a: RuntimeApp): [Point, string][] {
  const r = app.registry;
  const made: [Point, string][] = [];
  const rows = new Map((a.inline.tool_row ?? []).flatMap((decl) => toolNames(a.id, decl.tool).map((name) => [name, decl] as const)));
  if (rows.size) {
    const id = "inline.tool_row";
    const contribution: Contributions["tool.row"] = {
      id,
      match: (block) => rows.has(block.name),
      render: (block) => <InlineToolRow app={app} owner={a} decl={rows.get(block.name)!} block={block} />,
      title: (block) => inlineTitle(rows.get(block.name)!, block),
    };
    if (r.register("tool.row", contribution, a.id)) made.push(["tool.row", id]);
  }
  const kinds = new Map((a.inline.flow_block ?? []).map((decl) => [decl.kind, decl]));
  if (kinds.size) {
    const id = "inline.flow_block";
    const ns = `app.${a.id}`;
    const entryOf = (block: Block): Entry | null => {
      if (block.kind !== "custom" || (block as BlockOf<"custom">).type !== "entry") return null;
      const entry = (block as BlockOf<"custom">).data as Entry | null;
      return entry && entry.ns === ns && typeof entry.event === "string" && kinds.has(entry.event) ? entry : null;
    };
    const contribution: Contributions["flow.block"] = {
      id,
      match: (block) => entryOf(block) !== null,
      render: (block) => {
        const entry = entryOf(block)!;
        return <InlineCard key={entry.id} app={app} owner={a} decl={kinds.get(entry.event!)!} entry={entry} />;
      },
    };
    if (r.register("flow.block", contribution, a.id)) made.push(["flow.block", id]);
  }
  for (const decl of a.inline.command ?? []) {
    const contribution = command(app, a, decl);
    if (r.register("command", contribution, a.id)) made.push(["command", contribution.id]);
  }
  return made;
}

interface Candidate {
  replacement?: unknown;
  label?: Text;
  description?: Text;
  call?: { method?: unknown; params?: unknown };
  done?: Text;
}

/** One declared command: asked of the backend, and only candidates of its own name taken. */
function command(app: App, a: RuntimeApp, decl: CommandDecl): Contributions["command"] {
  const prefix = `/${decl.name}`;
  return {
    id: `inline.command.${decl.trigger}${decl.name}`,
    trigger: decl.trigger,
    async complete(query: string, ctx?: CommandContext): Promise<CommandCandidate[]> {
      const answer = await app.call<{ candidates?: Candidate[] } | null>(`app.${a.id}.complete`, {
        trigger: decl.trigger,
        name: decl.name,
        query,
        space: ctx?.space ?? null,
        session: ctx?.session ? { instance: ctx.session.instance, session_id: ctx.session.sessionId } : null,
      });
      const candidates = Array.isArray(answer?.candidates) ? answer!.candidates : [];
      const out: CommandCandidate[] = [];
      for (const c of candidates.slice(0, 50)) {
        if (typeof c?.replacement !== "string" || !c.replacement.trim()) continue;
        const replacement = c.replacement;
        // A `/` candidate is one of its own commands, or nothing: it does not take another's name.
        if (decl.trigger === "/" && !replacement.startsWith(prefix)) continue;
        const description = localized(c.description) || localized(decl.description);
        const candidate: CommandCandidate = {
          replacement,
          label: localized(c.label) || words(replacement).trim(),
          description,
          source: a.title,
        };
        const method = c.call?.method;
        if (decl.trigger === "/" && typeof method === "string" && method) {
          const params = c.call?.params && typeof c.call.params === "object" ? c.call.params : {};
          candidate.run = async () => {
            await app.call(`app.${a.id}.${method}`, params);
            app.notice(localized(c.done) || t("apps.command.done", { command: replacement.trim() }));
          };
        }
        out.push(candidate);
      }
      return out;
    },
  };
}

/** The left column's mark on a space an installed application brought: `owner` is the space's. */
export function AppMark({ app, owner }: { app: App; owner: string }) {
  const watch = appsOf(app);
  useSyncExternalStore(watch.subscribe, watch.getVersion);
  const a = watch.get(owner);
  if (!a || !watch.marked()) return null;
  const title = t("apps.mark.title", { name: a.title, trust: t(`apps.trust.${a.trust}`) });
  return <span className="app-mark" data-testid="space-app-mark" data-trust={a.trust} title={title} aria-label={title} />;
}
