/**
 * The runtime applications this page knows of, from the `apps` module's topic `harnix.apps` (the
 * value of `harnix.apps.list`, pushed when it changes): which are installed and how far each is
 * trusted — what the left column's mark says — and, for each one running, the inline points its
 * manifest declares (`runtime.inline`), which the shell draws for it.
 *
 * Also the reader's choice of whether the mark is drawn, kept in this browser.
 */

import type { App } from "../app.ts";
import type { Text } from "./template.ts";

export interface ToolRowDecl {
  tool: string;
  icon?: string;
  title?: Text;
  /** `result`, or `input.<path>`. */
  summary?: string;
  collapsed?: boolean;
}

export interface FieldDecl {
  label: Text;
  value: string;
}

export interface ButtonDecl {
  label: Text;
  method: string;
  params?: Record<string, unknown>;
  confirm?: Text;
  done?: Text;
}

export interface FlowBlockDecl {
  kind: string;
  icon?: string;
  title: Text;
  body?: Text;
  field?: FieldDecl[];
  button?: ButtonDecl[];
}

export interface CommandDecl {
  trigger: "/" | "@";
  name: string;
  description?: Text;
}

export interface GroupDecl {
  space?: string;
  by: "scene" | "field" | "method";
  field?: string;
  method?: string;
  other?: Text;
}

export interface Inline {
  tool_row?: ToolRowDecl[];
  flow_block?: FlowBlockDecl[];
  command?: CommandDecl[];
  sidebar_group?: GroupDecl[];
}

/** One installed application, as the shell draws for it. */
export interface RuntimeApp {
  id: string;
  title: string;
  /** `store`, `trusted` or `unsigned`. */
  trust: string;
  /** Enabled and running: its inline points are drawn. */
  running: boolean;
  inline: Inline;
}

interface Listed {
  id: string;
  title?: string | null;
  enabled?: boolean;
  trust?: string;
  runtime?: { state?: string; inline?: Inline | null } | null;
}

/** What the reader chose about the mark: `off` hides it. */
const MARK_KEY = "harnix.apps.mark";

export class AppsWatch {
  private apps = new Map<string, RuntimeApp>();
  private listeners = new Set<() => void>();
  private version = 0;
  private hidden = readMark();

  /** A value of `harnix.apps`. */
  take(value: unknown): void {
    const listed = ((value as { apps?: Listed[] } | null)?.apps ?? []).filter((a) => a && typeof a.id === "string");
    this.apps = new Map(
      listed.map((a) => [
        a.id,
        {
          id: a.id,
          title: a.title || a.id,
          trust: a.trust ?? "unsigned",
          running: Boolean(a.enabled) && a.runtime?.state === "running",
          inline: (a.runtime?.inline as Inline | null | undefined) ?? {},
        },
      ]),
    );
    this.changed();
  }

  get(id: string): RuntimeApp | undefined {
    return this.apps.get(id);
  }

  running(): RuntimeApp[] {
    return [...this.apps.values()].filter((a) => a.running);
  }

  /** Whether the left column marks applications' spaces. */
  marked(): boolean {
    return !this.hidden;
  }

  setMarked(on: boolean): void {
    this.hidden = !on;
    try {
      if (on) localStorage.removeItem(MARK_KEY);
      else localStorage.setItem(MARK_KEY, "off");
    } catch {
      // kept for this page only
    }
    this.changed();
  }

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  readonly getVersion = (): number => this.version;

  private changed(): void {
    this.version += 1;
    for (const fn of [...this.listeners]) fn();
  }
}

function readMark(): boolean {
  try {
    return localStorage.getItem(MARK_KEY) === "off";
  } catch {
    return false;
  }
}

const watches = new WeakMap<App, AppsWatch>();

/** The page's one watch. */
export function appsOf(app: App): AppsWatch {
  let watch = watches.get(app);
  if (!watch) {
    watch = new AppsWatch();
    watches.set(app, watch);
  }
  return watch;
}

/** The names a session knows an application's tool by: in this process, and in a spawned daemon. */
export function toolNames(id: string, tool: string): string[] {
  const flat = id.replace(/\./g, "_");
  return [`app__${flat}__${tool}`, `mcp__app-${flat}__${tool}`];
}
