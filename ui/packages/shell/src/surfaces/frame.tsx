/**
 * One surface: the application's page in its sandboxed frame, and the frame the shell draws
 * around it — its title while it loads, what went wrong and a reload when it fails, a note when it
 * has been set aside or its application has stopped. A failed page is "this part failed, reload",
 * never half a white panel.
 *
 * The `<iframe>` is made here by hand, not by React, so its `sandbox` is set before it has a
 * `src`: `allow-scripts allow-forms`, and nothing else — no `allow-same-origin` (an opaque origin:
 * the page's token, storage and cookies are out of reach), no top navigation, no popups (a web
 * page opens through the bridge's `openLink`, which the shell asks the person about). Forms are
 * allowed because without it a form's `submit` event never fires, and a page's own forms are how
 * it takes input; its policy's `form-action 'none'` is what keeps one from submitting anywhere. The
 * frame can still navigate itself: the bridge empties it when it does (`bridge.ts`).
 */

import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import { useT } from "@harnix/react";
import { inDialog } from "@harnix/app-sdk/protocol";

import type { App } from "../app.ts";
import { FrameBridge } from "./bridge.ts";
import { asking, onAsking } from "./confirm.tsx";
import { autoHeight, surfaceKey, titleOf, type AppInterface, type SurfaceDecl } from "./model.ts";
import { surfaces } from "./store.ts";

/** What a surface's frame may have the shell do beyond the bridge's own requests. */
export interface SurfaceActions {
  /** The interface space its sessions are drawn in. */
  space: string | null;
  openModal(id: string, params: unknown): Promise<unknown>;
  secondary(open: { params: unknown; title?: string } | null): void;
  badge(value: string | number | null): void;
}

/** The tallest an auto-height surface grows before it scrolls inside. */
function ceiling(point: string): number {
  if (point === "home.card") return 480;
  if (point === "modal") return Math.round(window.innerHeight * 0.72);
  return 4000;
}

export function SurfaceView({
  app,
  iface,
  decl,
  actions,
  params,
  visible = true,
  onClose,
}: {
  app: App;
  iface: AppInterface;
  decl: SurfaceDecl;
  actions: SurfaceActions;
  params?: unknown;
  /** Whether its place is on screen (a kept surface's, from the keeper). */
  visible?: boolean;
  /** A modal's: the frame closed itself, answering this. */
  onClose?: (result: unknown) => void;
}) {
  const t = useT();
  const key = surfaceKey(decl);
  const status = useSyncExternalStore(surfaces.subscribe, () => surfaces.get(key));
  const stopped = useSyncExternalStore(surfaces.subscribe, () => surfaces.isStopped(decl.app));
  const rev = useSyncExternalStore(surfaces.subscribe, () => surfaces.rev(decl.app));
  const dialog = useSyncExternalStore(app.subscribe, () => app.snapshot.modal != null);
  const question = useSyncExternalStore(onAsking, asking);
  const [generation, setGeneration] = useState(0);
  const [height, setHeight] = useState<number | null>(null);
  const holder = useRef<HTMLDivElement>(null);
  const bridge = useRef<FrameBridge | null>(null);
  const frame = useRef<HTMLIFrameElement | null>(null);
  const latest = useRef({ params, onClose, actions });
  latest.current = { params, onClose, actions };
  const state = stopped ? "stopped" : (status?.state ?? "loading");
  const live = !stopped && status?.state !== "failed" && status?.state !== "aside" && decl.frame !== null;
  const sized = autoHeight(decl.point);
  const title = titleOf(iface, decl);
  // A dialog of the shell over this frame (one it is not inside), or the shell's question.
  const covered = question || (dialog && !inDialog(decl.point));

  useLayoutEffect(() => {
    const at = holder.current;
    if (!live || !at || !decl.frame) return;
    if (!surfaces.get(key)) surfaces.set({ key, app: decl.app, point: decl.point, id: decl.id, state: "loading" });
    const iframe = document.createElement("iframe");
    iframe.setAttribute("sandbox", "allow-scripts allow-forms");
    iframe.setAttribute("referrerpolicy", "no-referrer");
    iframe.setAttribute("title", title);
    iframe.className = "hx-frame";
    iframe.dataset.app = decl.app;
    iframe.dataset.surface = decl.id;
    iframe.dataset.point = decl.point;
    iframe.dataset.testid = "surface-frame";
    const b = new FrameBridge(iframe, {
      app,
      iface,
      decl,
      space: latest.current.actions.space,
      params: latest.current.params ?? null,
      ready: () => surfaces.set({ key, app: decl.app, point: decl.point, id: decl.id, state: "ready" }),
      failed: (reason) => {
        surfaces.fail({ key, app: decl.app, point: decl.point, id: decl.id, reason });
        app.registry.note({ point: "surface", owner: decl.app, id: `${decl.point}:${decl.id}`, reason });
      },
      resize: (h) => setHeight(Math.max(24, Math.min(ceiling(decl.point), Math.ceil(h)))),
      close: (result) => latest.current.onClose?.(result),
      openModal: (id, p) => latest.current.actions.openModal(id, p),
      secondary: (open) => latest.current.actions.secondary(open),
      badge: (value) => latest.current.actions.badge(value),
    });
    bridge.current = b;
    frame.current = iframe;
    iframe.src = decl.frame;
    at.append(iframe);
    return () => {
      b.dispose();
      iframe.remove();
      bridge.current = null;
      frame.current = null;
      surfaces.gone(key);
    };
    // A new generation is a reload; the rest is read through `latest`.
  }, [generation, rev, live, key]);

  // New params go to the frame that has them, rather than making it again.
  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    bridge.current?.setParams(params ?? null);
  }, [JSON.stringify(params ?? null)]);

  useEffect(() => {
    if (frame.current) frame.current.inert = covered;
    bridge.current?.setLocal({ visible, modal: covered });
  }, [covered, visible, generation, live]);

  const reload = (fresh: boolean) => {
    if (fresh) surfaces.retry(key);
    else surfaces.reload(key);
    setHeight(null);
    setGeneration((g) => g + 1);
  };

  return (
    <div
      className={`hx-surface${sized ? " sized" : " fill"}`}
      data-testid="surface"
      data-app={decl.app}
      data-point={decl.point}
      data-surface={decl.id}
      data-state={state}
      style={sized && height !== null && live ? { height } : undefined}
    >
      <div className="hx-frame-holder" ref={holder} />
      {state === "loading" && live ? (
        <div className="hx-surface-cover loading" data-testid="surface-loading">
          <span className="hx-spin" aria-hidden="true" />
          <span>{t("surface.loading", { title })}</span>
        </div>
      ) : null}
      {state === "failed" ? (
        <div className="hx-surface-cover failed" role="alert" data-testid="surface-failed">
          <div className="hx-surface-title">{t("surface.failed.title", { title })}</div>
          {status?.reason ? <div className="hx-surface-why" data-testid="surface-reason">{status.reason}</div> : null}
          <button type="button" className="btn sm outline" data-testid="surface-reload" onClick={() => reload(false)}>
            {t("surface.reload")}
          </button>
        </div>
      ) : null}
      {state === "aside" ? (
        <div className="hx-surface-cover failed" role="alert" data-testid="surface-aside">
          <div className="hx-surface-title">{t("surface.aside.title", { title })}</div>
          <div className="hx-surface-why">{t("surface.aside.body")}</div>
          <button type="button" className="btn sm outline" data-testid="surface-retry" onClick={() => reload(true)}>
            {t("surface.aside.retry")}
          </button>
        </div>
      ) : null}
      {state === "stopped" ? (
        <div className="hx-surface-cover" data-testid="surface-stopped">
          <div className="hx-surface-why">{t("surface.stopped", { app: titleOf(iface) })}</div>
        </div>
      ) : null}
    </div>
  );
}
