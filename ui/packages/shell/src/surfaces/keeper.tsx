/**
 * The surfaces that outlive the place they are shown in: a space's centre and left column (made
 * when the space is first entered, kept while it is away) and its right-column tabs (made when
 * first selected, kept after). A frame moved in the document is a frame reloaded, so these are
 * never in the shell's own tree: each lives in a host of its own, under `<body>`, and is laid over
 * the place it is shown in — the slot the shell's contribution draws (`KeptSlot`) — following it
 * every frame while it is there, and hidden (still alive, its state kept) while it is not.
 *
 * Laid over, a host takes the stacking of where its slot is (the highest `z-index` among the
 * slot's positioned ancestors), so what the shell draws above that place — its menus, a panel
 * lying over the conversation on a narrow window, its dialogs — is drawn above the frame too. It
 * is clipped to what of the slot its scrolling ancestors show.
 *
 * Gone with the application: when it stops, or the page goes.
 */

import { useLayoutEffect, useRef, useSyncExternalStore } from "react";

import type { App } from "../app.ts";
import { SurfaceView, type SurfaceActions } from "./frame.tsx";
import { surfaceKey, type AppInterface, type SurfaceDecl } from "./model.ts";

interface Kept {
  key: string;
  app: App;
  iface: AppInterface;
  decl: SurfaceDecl;
  actions: SurfaceActions;
  slot: HTMLElement | null;
  visible: boolean;
  z: number;
}

class Keeper {
  private readonly kept = new Map<string, Kept>();
  private readonly listeners = new Set<() => void>();
  private list: Kept[] = [];
  private frame = 0;
  readonly hosts = new Map<string, HTMLDivElement>();

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  readonly all = (): Kept[] => this.list;

  /** A slot is on the page: its surface is made if it was not, and laid over it. */
  attach(entry: Omit<Kept, "key" | "slot" | "visible" | "z">, slot: HTMLElement): void {
    const key = surfaceKey(entry.decl);
    const was = this.kept.get(key);
    this.kept.set(key, { ...(was ?? entry), actions: entry.actions, key, slot, visible: was?.visible ?? false, z: stackOf(slot) });
    this.changed();
    this.track();
  }

  /** The slot left the page: its surface stays, hidden, until it is shown again. */
  detach(key: string, slot: HTMLElement): void {
    const kept = this.kept.get(key);
    if (!kept || kept.slot !== slot) return;
    this.kept.set(key, { ...kept, slot: null, visible: false });
    this.changed();
  }

  /** The application stopped: its kept surfaces go. */
  drop(app: string): void {
    let any = false;
    for (const [key, kept] of this.kept) {
      if (kept.decl.app !== app) continue;
      this.kept.delete(key);
      any = true;
    }
    if (any) this.changed();
  }

  /** Lay each host over its slot; runs every animation frame while any slot is on the page. */
  private readonly place = (): void => {
    this.frame = 0;
    let attached = false;
    let moved = false;
    for (const kept of this.kept.values()) {
      const host = this.hosts.get(kept.key);
      const here = kept.slot !== null && kept.slot.isConnected;
      // Not drawn yet: the next frame has it.
      if (here) attached = true;
      if (!host) continue;
      if (!here) {
        host.style.visibility = "hidden";
        host.style.pointerEvents = "none";
        if (kept.visible) {
          kept.visible = false;
          moved = true;
        }
        continue;
      }
      const r = kept.slot!.getBoundingClientRect();
      const clip = clipOf(kept.slot!, r);
      const shown = r.width > 1 && r.height > 1 && clip.width > 0 && clip.height > 0;
      host.style.left = `${r.left}px`;
      host.style.top = `${r.top}px`;
      host.style.width = `${r.width}px`;
      host.style.height = `${r.height}px`;
      host.style.zIndex = String(kept.z);
      host.style.clipPath = `inset(${clip.top - r.top}px ${r.right - clip.right}px ${r.bottom - clip.bottom}px ${clip.left - r.left}px)`;
      host.style.visibility = shown ? "visible" : "hidden";
      host.style.pointerEvents = shown ? "auto" : "none";
      if (shown !== kept.visible) {
        kept.visible = shown;
        moved = true;
      }
    }
    if (moved) {
      this.list = [...this.kept.values()];
      for (const fn of [...this.listeners]) fn();
    }
    if (attached) this.track();
  };

  private track(): void {
    if (!this.frame) this.frame = requestAnimationFrame(this.place);
  }

  private changed(): void {
    this.list = [...this.kept.values()];
    for (const fn of [...this.listeners]) fn();
  }
}

/** The page's kept surfaces. */
export const keeper = new Keeper();

/** The highest `z-index` among an element's positioned ancestors: the stacking it is drawn in. */
function stackOf(el: HTMLElement): number {
  let z = 1;
  for (let at: HTMLElement | null = el.parentElement; at; at = at.parentElement) {
    const style = getComputedStyle(at);
    const n = Number.parseInt(style.zIndex, 10);
    if (style.position !== "static" && Number.isFinite(n)) z = Math.max(z, n);
  }
  return z;
}

/** What of `rect` its scrolling (or clipping) ancestors leave visible. */
function clipOf(el: HTMLElement, rect: DOMRect): { left: number; top: number; right: number; bottom: number; width: number; height: number } {
  let { left, top, right, bottom } = rect;
  for (let at: HTMLElement | null = el.parentElement; at && at !== document.body; at = at.parentElement) {
    const style = getComputedStyle(at);
    if (style.overflowX === "visible" && style.overflowY === "visible") continue;
    const r = at.getBoundingClientRect();
    left = Math.max(left, r.left);
    top = Math.max(top, r.top);
    right = Math.min(right, r.right);
    bottom = Math.min(bottom, r.bottom);
  }
  left = Math.max(left, 0);
  top = Math.max(top, 0);
  right = Math.min(right, window.innerWidth);
  bottom = Math.min(bottom, window.innerHeight);
  return { left, top, right, bottom, width: Math.max(0, right - left), height: Math.max(0, bottom - top) };
}

/** Where a kept surface is shown, in the shell's own tree: an empty place it is laid over. */
export function KeptSlot({ app, iface, decl, actions }: { app: App; iface: AppInterface; decl: SurfaceDecl; actions: SurfaceActions }) {
  const slot = useRef<HTMLDivElement>(null);
  const key = surfaceKey(decl);
  useLayoutEffect(() => {
    const el = slot.current;
    if (!el) return;
    keeper.attach({ app, iface, decl, actions }, el);
    return () => keeper.detach(key, el);
  }, [key]);
  return <div ref={slot} className={`hx-slot ${decl.point.replace(".", "-")}`} data-testid="surface-slot" data-app={decl.app} data-surface={decl.id} data-point={decl.point} />;
}

/** The kept surfaces, each in its host: rendered once for the page, outside the shell's tree. */
export function KeeperLayer() {
  const kept = useSyncExternalStore(keeper.subscribe, keeper.all);
  return (
    <>
      {kept.map((k) => (
        <KeptHost key={k.key} kept={k} />
      ))}
    </>
  );
}

function KeptHost({ kept }: { kept: Kept }) {
  return (
    <div
      className="hx-kept"
      data-testid="surface-kept"
      data-app={kept.decl.app}
      data-point={kept.decl.point}
      data-surface={kept.decl.id}
      ref={(el) => {
        if (el) keeper.hosts.set(kept.key, el);
        else keeper.hosts.delete(kept.key);
      }}
    >
      <SurfaceView app={kept.app} iface={kept.iface} decl={kept.decl} actions={kept.actions} visible={kept.visible} />
    </div>
  );
}
