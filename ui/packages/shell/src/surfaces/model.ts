/**
 * What the page is told of the applications whose interface runs in isolated frames: their
 * surfaces (`[[surface]]`), what their frames may reach (`[interface]`), and the in-page modules
 * that will not run because of who signed them. Read off hello's `contributions`, where the
 * backend lists each running application's interface — modules for a trusted publisher's, which
 * `packages.ts` imports; surfaces for everyone else's, which this directory draws.
 */

import type { Hello } from "@harnix/core";
import { currentLocale } from "@harnix/react";
import type { SurfacePoint } from "@harnix/app-sdk/protocol";

/** One application drawn in frames. */
export interface AppInterface {
  id: string;
  title: string | null;
  trust: string;
  /** `[interface] hub`: what its frames may reach beyond `app.<id>.*`. */
  hub: string[];
  links: boolean;
  /** Its backend spaces (`[[space]]`), in order: where its frames' new sessions land. */
  spaces: string[];
  surfaces: SurfaceDecl[];
}

/** One `[[surface]]`, as the backend tells it. */
export interface SurfaceDecl {
  app: string;
  point: SurfacePoint;
  id: string;
  /** Its page, `/apps/<id>/ui/<page>`; null for a space whose centre is the conversation. */
  frame: string | null;
  title: Record<string, string> | null;
  space: string | null;
  /** The backend space a space's sessions are in. */
  sessions: string | null;
  icon: string | null;
  order: number | null;
  center: "hidden" | "minimized" | "chat" | null;
  kinds: string[];
  wide: boolean;
}

/** A module that runs in the page, refused for an application that is not a trusted publisher's. */
export interface RefusedModule {
  app: string;
  point: string;
  module: string;
  reason: string;
}

const POINTS = new Set<SurfacePoint>(["space", "space.sidebar", "space.tab", "space.secondary", "details.panel", "settings.section", "home.card", "modal"]);

interface Entry {
  plugin?: string;
  point?: string;
  frame?: string | null;
  refused?: string;
  reason?: string;
  surface?: Partial<Omit<SurfaceDecl, "app" | "point" | "frame">>;
  interface?: { hub?: string[]; links?: boolean; trust?: string; title?: string | null; spaces?: string[] };
}

const strings = (v: unknown): string[] => (Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : []);

/** The applications hello lists surfaces for, and the modules it says will not run. */
export function interfacesOf(hello: Pick<Hello, "contributions">): { apps: AppInterface[]; refused: RefusedModule[] } {
  const entries = (Array.isArray(hello.contributions) ? hello.contributions : []) as Entry[];
  const apps = new Map<string, AppInterface>();
  const refused: RefusedModule[] = [];
  for (const entry of entries) {
    const id = entry.plugin;
    if (typeof id !== "string") continue;
    if (typeof entry.refused === "string") {
      refused.push({ app: id, point: entry.point ?? "?", module: entry.refused, reason: entry.reason ?? "" });
      continue;
    }
    const s = entry.surface;
    if (!s || !POINTS.has(entry.point as SurfacePoint)) continue;
    const i = entry.interface ?? {};
    let app = apps.get(id);
    if (!app) {
      app = { id, title: i.title ?? null, trust: i.trust ?? "store", hub: strings(i.hub), links: Boolean(i.links), spaces: strings(i.spaces), surfaces: [] };
      apps.set(id, app);
    }
    // A frame is only ever this application's own files.
    const frame = typeof entry.frame === "string" && entry.frame.startsWith(`/apps/${id}/ui/`) ? entry.frame : null;
    app.surfaces.push({
      app: id,
      point: entry.point as SurfacePoint,
      id: typeof s.id === "string" ? s.id : String(entry.point),
      frame,
      title: s.title && typeof s.title === "object" ? (s.title as Record<string, string>) : null,
      space: typeof s.space === "string" ? s.space : null,
      sessions: typeof s.sessions === "string" ? s.sessions : null,
      icon: typeof s.icon === "string" ? s.icon : null,
      order: typeof s.order === "number" ? s.order : null,
      center: s.center === "minimized" || s.center === "chat" || s.center === "hidden" ? s.center : null,
      kinds: strings(s.kinds),
      wide: Boolean(s.wide),
    });
  }
  return { apps: [...apps.values()], refused };
}

/** What a surface is called here: its own title in this language, else in English, else the app's. */
export function titleOf(app: AppInterface, decl?: SurfaceDecl): string {
  const titles = decl?.title;
  return titles?.[currentLocale()] ?? titles?.en ?? Object.values(titles ?? {})[0] ?? app.title ?? app.id;
}

/** A surface's identity on this page: one frame per key. */
export function surfaceKey(decl: SurfaceDecl): string {
  return `${decl.app}/${decl.point}/${decl.space ?? ""}/${decl.id}`;
}

/** Whether the shell sizes a point's frame to its content. */
export function autoHeight(point: SurfacePoint): boolean {
  return point === "home.card" || point === "settings.section" || point === "modal";
}
