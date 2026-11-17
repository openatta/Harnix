/**
 * The models a provider's endpoint serves, asked through the `settings` extension
 * (`settings.listModels`, the engine's `config.listModels`, which uses the provider's own
 * credentials and base URL). Asked once per (scene, project, provider) and kept for as long as the
 * page is open, since every model field of a section would ask the same; `fresh` asks again. A
 * failure is not kept: the next field to ask tries again.
 */

import type { HostApi, ModelLister } from "@harnix/react";

type Where = { scene?: string; project_root: string | null };

/** What each host's page asked, and its listers: a page has one host. */
const pages = new WeakMap<HostApi, { asked: Map<string, Promise<string[]>>; listers: Map<string, ModelLister> }>();

function page(host: HostApi) {
  let kept = pages.get(host);
  if (!kept) {
    kept = { asked: new Map(), listers: new Map() };
    pages.set(host, kept);
  }
  return kept;
}

/** The ids an answer lists, each once. */
export function modelIds(answer: unknown): string[] {
  const rows = answer && typeof answer === "object" ? (answer as { models?: unknown }).models : undefined;
  if (!Array.isArray(rows)) return [];
  const ids = rows.map((row) => (row && typeof row === "object" ? (row as { id?: unknown }).id : row)).filter((id): id is string => typeof id === "string" && id.trim() !== "");
  return [...new Set(ids)];
}

/** One lister per (scene, project) and host, the same function each time it is asked for. */
export function modelLister(host: HostApi, where: Where): ModelLister {
  const { asked, listers } = page(host);
  const at = `${where.scene ?? ""}\u0000${where.project_root ?? ""}`;
  const known = listers.get(at);
  if (known) return known;
  const lister: ModelLister = (provider, fresh) => {
    const key = `${at}\u0000${provider ?? ""}`;
    const kept = asked.get(key);
    if (kept && !fresh) return kept;
    const request = {
      ...(where.scene ? { scene: where.scene } : {}),
      ...(where.project_root ? { project_root: where.project_root } : {}),
      ...(provider ? { provider } : {}),
    };
    const asking = host.call("settings.listModels", request).then(modelIds);
    asked.set(key, asking);
    asking.catch(() => {
      if (asked.get(key) === asking) asked.delete(key);
    });
    return asking;
  };
  listers.set(at, lister);
  return lister;
}

/** Forget what was asked: a provider changed, and what its endpoint serves may have. */
export function forgetModels(host: HostApi): void {
  page(host).asked.clear();
}
