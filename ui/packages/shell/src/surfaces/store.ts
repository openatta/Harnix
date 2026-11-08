/**
 * How this page's frames are doing: each surface's state, the failures counted against it, the
 * bridge requests refused to each application. Page-local by choice — the backend cannot see a
 * frame, and each window has frames of its own — so the system space's Apps tab reads it from
 * here (`status.tsx`) beside what the backend reports of the application's VM, and refusals go to
 * Settings → Diagnostics through the registry as well.
 *
 * Failures: a frame that fails is shown as failed, with a reload. `ASIDE_AFTER` failures within
 * `ASIDE_WITHIN` set the surface aside: it is not made again until someone asks for it on purpose
 * ("try again"), as a contribution that fails is left out rather than retried at every draw.
 */

export const ASIDE_AFTER = 3;
export const ASIDE_WITHIN = 5 * 60_000;
const KEEP_REFUSED = 20;

export type SurfaceState = "loading" | "ready" | "failed" | "aside" | "stopped";

export interface SurfaceStatus {
  key: string;
  app: string;
  point: string;
  id: string;
  state: SurfaceState;
  reason?: string;
}

export interface BridgeRefusal {
  name: string;
  reason: string;
  at: number;
}

class SurfaceStore {
  private readonly status = new Map<string, SurfaceStatus>();
  private readonly failures = new Map<string, number[]>();
  private readonly crashes = new Map<string, number>();
  private readonly refusals = new Map<string, BridgeRefusal[]>();
  private readonly stopped = new Set<string>();
  private readonly revs = new Map<string, number>();
  private readonly listeners = new Set<() => void>();
  private version = 0;

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  readonly getVersion = (): number => this.version;

  /** A surface's state, as its frame says; undefined if it has none here. */
  get(key: string): SurfaceStatus | undefined {
    return this.status.get(key);
  }

  set(status: SurfaceStatus): void {
    const was = this.status.get(status.key);
    if (was && was.state === status.state && was.reason === status.reason) return;
    this.status.set(status.key, status);
    this.changed();
  }

  /** The frame is gone (its surface was left): it is no longer live. A failure stays on the record. */
  gone(key: string): void {
    const was = this.status.get(key);
    if (!was || (was.state !== "loading" && was.state !== "ready")) return;
    this.status.delete(key);
    this.changed();
  }

  /** Reload a failed surface: its failures still count. */
  reload(key: string): void {
    const was = this.status.get(key);
    if (!was) return;
    this.status.set(key, { ...was, state: "loading", reason: undefined });
    this.changed();
  }

  /** A failure: `aside` when it is one too many. */
  fail(status: Omit<SurfaceStatus, "state">, now = Date.now()): "failed" | "aside" {
    const recent = (this.failures.get(status.key) ?? []).filter((at) => now - at < ASIDE_WITHIN);
    recent.push(now);
    this.failures.set(status.key, recent);
    this.crashes.set(status.app, (this.crashes.get(status.app) ?? 0) + 1);
    const state = recent.length >= ASIDE_AFTER ? "aside" : "failed";
    this.status.set(status.key, { ...status, state });
    this.changed();
    return state;
  }

  /** Someone asked for a surface set aside on purpose: its count starts again. */
  retry(key: string): void {
    this.failures.delete(key);
    const was = this.status.get(key);
    if (was) this.status.set(key, { ...was, state: "loading", reason: undefined });
    this.changed();
  }

  isAside(key: string): boolean {
    return this.status.get(key)?.state === "aside";
  }

  refuse(app: string, name: string, reason: string, now = Date.now()): void {
    const list = this.refusals.get(app) ?? [];
    list.unshift({ name, reason, at: now });
    list.length = Math.min(list.length, KEEP_REFUSED);
    this.refusals.set(app, list);
    this.changed();
  }

  /** The application stopped (disabled, uninstalled, set aside by its VM): its frames go. */
  setStopped(app: string, stopped: boolean): void {
    if (stopped === this.stopped.has(app)) return;
    if (stopped) this.stopped.add(app);
    else this.stopped.delete(app);
    this.changed();
  }

  /** How many times an application's frames were made again from its files (dev's hot reload). */
  rev(app: string): number {
    return this.revs.get(app) ?? 0;
  }

  bump(app: string): void {
    this.revs.set(app, this.rev(app) + 1);
    this.changed();
  }

  isStopped(app: string): boolean {
    return this.stopped.has(app);
  }

  /** What the Apps tab shows of one application. */
  summary(app: string): { live: number; crashes: number; surfaces: SurfaceStatus[]; refused: BridgeRefusal[] } {
    const surfaces = [...this.status.values()].filter((s) => s.app === app);
    return {
      live: surfaces.filter((s) => s.state === "ready" || s.state === "loading").length,
      crashes: this.crashes.get(app) ?? 0,
      surfaces,
      refused: this.refusals.get(app) ?? [],
    };
  }

  private changed(): void {
    this.version += 1;
    for (const fn of [...this.listeners]) fn();
  }
}

/** This page's frames. */
export const surfaces = new SurfaceStore();
