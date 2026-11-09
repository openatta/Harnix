/**
 * What the composer offers: the `/` menu's commands and the `@` menu's mentions — the shell's own
 * (`@session:`) and the `command` contributions' — and the references a sent message carries.
 */

import { useEffect, useState, useSyncExternalStore } from "react";
import { errorText } from "@harnix/core";
import { useRegistry, type ChatCommand, type CommandContext, type MentionItem } from "@harnix/react";

import type { App } from "./app.ts";
import { spacesOf } from "./Sidebar.tsx";

/** `@session:<instance>/<id>` in a message: the sessions it refers to. */
export function referencesIn(text: string): Record<string, unknown> {
  const references = [...text.matchAll(/@session:([\w.-]+)\/([\w-]+)/g)].map((m) => ({ instance: m[1], session_id: m[2] }));
  return references.length ? { references } : {};
}

/** Sessions of every space whose name matches, for `@session:`. */
export async function sessionMentions(app: App, query: string): Promise<MentionItem[]> {
  const { instances } = await app.call<{ instances: { id: string; status: { state: string } }[] }>("harnix.instances.list", {});
  const q = query.toLowerCase();
  const items: MentionItem[] = [];
  for (const instance of instances.filter((i) => i.status.state === "ready")) {
    const { sessions } = await app
      .call<{ sessions: { session_id: string; name?: string | null; space?: string }[] }>("harnix.sessions", { instance: instance.id })
      .catch(() => ({ sessions: [] }));
    for (const row of sessions) {
      const name = row.name || row.session_id;
      if (q && !name.toLowerCase().includes(q) && !row.session_id.startsWith(q)) continue;
      items.push({
        id: `session:${instance.id}/${row.session_id}`,
        label: `${name} · ${row.space ?? "sessions"}`,
        insert: `@session:${instance.id}/${row.session_id} `,
      });
    }
  }
  return items.slice(0, 20);
}

/** Where the composer is: the front space and its open session (`CommandContext`). */
function commandContext(app: App): CommandContext {
  const current = app.snapshot.current;
  return { space: app.snapshot.space, session: current ? { instance: current.instance, sessionId: current.sessionId } : null };
}

/** The `command` contributions a menu asks: `/` for those without a trigger. */
function commandsFor(app: App, trigger: "/" | "@") {
  return app.registry.at("command").filter((c) => (c.trigger ?? "/") === trigger);
}

/**
 * The `/` menu: every `/` contribution's candidates for an empty query, in order, asked again when
 * the registry, the engine's commands, the front space or its session change (`ctx` follows them).
 * The front space's `hideCommands` leave out other owners' commands of those names, so its own
 * command of the same name is the one listed; the first of a name wins otherwise.
 */
export function useCommands(app: App): ChatCommand[] {
  const registry = useRegistry(app.registry);
  const hello = useSyncExternalStore(app.subscribe, () => app.snapshot.hello);
  const front = useSyncExternalStore(app.subscribe, () => app.snapshot.space);
  const current = useSyncExternalStore(app.subscribe, () => app.snapshot.current);
  const [commands, setCommands] = useState<ChatCommand[]>([]);
  useEffect(() => {
    let alive = true;
    const ctx = commandContext(app);
    const space = spacesOf(app).find((c) => c.id === ctx.space);
    const hidden = new Set(space?.hideCommands ?? []);
    const asked = commandsFor(app, "/");
    void Promise.all(
      asked.map((c) =>
        Promise.resolve()
          .then(() => c.complete("", ctx))
          .catch((e: unknown) => {
            registry.fail("command", c, e);
            return [];
          }),
      ),
    ).then((lists) => {
      if (!alive) return;
      const seen = new Set<string>();
      const out: ChatCommand[] = [];
      lists.forEach((list, i) => {
        const contribution = asked[i]!;
        for (const candidate of list) {
          const name = candidate.replacement.replace(/^\//, "").trim();
          if (!name || seen.has(name)) continue;
          if (hidden.has(name) && contribution.owner !== space?.owner) continue;
          seen.add(name);
          const run = candidate.run;
          out.push({
            name,
            description: candidate.description,
            ...(run
              ? {
                  run: () =>
                    Promise.resolve()
                      .then(() => run.call(candidate, ctx))
                      .catch((e: unknown) => app.notice(errorText(e), "error")),
                }
              : {}),
          });
        }
      });
      setCommands(out.slice(0, 200));
    });
    return () => {
      alive = false;
    };
  }, [app, registry, hello, front, current, registry.getVersion()]);
  return commands;
}

/** At most this many `@` candidates, the shell's own and the contributions' together. */
const MENTIONS_MAX = 20;
/** A contribution slower than this loses its `@` candidates for this keystroke. */
const MENTION_WAIT_MS = 1500;

/**
 * The `@` menu: the shell's own candidates (files, `@session:`) first, then each `@` contribution's
 * for the same query, in order of registration — or those first, when the front space says
 * `mention: { first: "contributed" }`. One that throws or is slow loses its own.
 */
export async function withContributed(app: App, query: string, own: MentionItem[]): Promise<MentionItem[]> {
  const asked = commandsFor(app, "@");
  if (!asked.length) return own;
  const ctx = commandContext(app);
  const lists = await Promise.all(
    asked.map((c) =>
      Promise.race([
        Promise.resolve().then(() => c.complete(query, ctx)),
        new Promise<never>((_, reject) => setTimeout(() => reject(new Error(`no answer in ${MENTION_WAIT_MS} ms`)), MENTION_WAIT_MS)),
      ]).catch((e: unknown) => {
        app.registry.fail("command", c, e);
        return [];
      }),
    ),
  );
  const contributed = lists.flat().map((candidate, i) => ({
    id: candidate.label || candidate.replacement || String(i),
    label: candidate.description,
    insert: candidate.replacement,
  }));
  const first = spacesOf(app).find((c) => c.id === app.snapshot.space)?.mention?.first;
  return (first === "contributed" ? [...contributed, ...own] : [...own, ...contributed]).slice(0, MENTIONS_MAX);
}
