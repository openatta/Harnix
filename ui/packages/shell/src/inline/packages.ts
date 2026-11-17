/**
 * The extension packages this page knows of, from the `packages` extension's topic `packages.list` (the
 * value of `packages.list`, pushed when it changes): which are installed and how far each is
 * trusted — what the left column's mark says — and, for each one running, the inline points its
 * manifest declares (`runtime.inline`), which the shell draws for it.
 *
 * Also the reader's choice of whether the mark is drawn, kept in this browser.
 */

import type { Button, Command, Field, FlowBlock, Inline, PackagesList, SidebarGroup, ToolRow } from "@harnix/core";

import type { App } from "../app.ts";

/** The manifest's `[inline]` declarations, as `packages.list` carries them: generated from the Rust that reads the manifest. */
export type { Inline };
export type ToolRowDecl = ToolRow;
export type FieldDecl = Field;
export type ButtonDecl = Button;
export type FlowBlockDecl = FlowBlock;
export type CommandDecl = Command;
export type GroupDecl = SidebarGroup;

/** One installed package, as the shell draws for it. */
export interface RuntimePackage {
  id: string;
  title: string;
  /** `store`, `trusted` or `unsigned`. */
  trust: string;
  /** Enabled and running: its inline points are drawn. */
  running: boolean;
  inline: Inline;
}

/** What the reader chose about the mark: `off` hides it. */
const MARK_KEY = "packages.mark";

export class PackagesWatch {
  private packages = new Map<string, RuntimePackage>();
  private listeners = new Set<() => void>();
  private version = 0;
  private hidden = readMark();

  /** A value of `packages.list`. */
  take(value: unknown): void {
    const listed = ((value as PackagesList | null)?.packages ?? []).filter((a) => a && typeof a.id === "string");
    this.packages = new Map(
      listed.map((a) => [
        a.id,
        {
          id: a.id,
          title: a.title || a.id,
          trust: a.trust ?? "unsigned",
          running: Boolean(a.enabled) && a.runtime?.state === "running",
          inline: a.runtime?.inline ?? {},
        },
      ]),
    );
    this.changed();
  }

  get(id: string): RuntimePackage | undefined {
    return this.packages.get(id);
  }

  running(): RuntimePackage[] {
    return [...this.packages.values()].filter((a) => a.running);
  }

  /** Whether the left column marks packages' spaces. */
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

const watches = new WeakMap<App, PackagesWatch>();

/** The page's one watch. */
export function packagesOf(app: App): PackagesWatch {
  let watch = watches.get(app);
  if (!watch) {
    watch = new PackagesWatch();
    watches.set(app, watch);
  }
  return watch;
}

/** The names a session knows a package's tool by (`harnix_package::tool_name`): the same in every session, of any instance. */
export function toolNames(id: string, tool: string): string[] {
  return [`ext__${id.replace(/\./g, "_")}__${tool}`];
}
