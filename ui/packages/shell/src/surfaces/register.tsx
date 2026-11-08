/**
 * The applications whose interface runs in isolated frames, put on the page: each surface its
 * application declares becomes the contribution the shell already knows how to place — a space
 * (its entry, centre, left column, tabs, second panel), a `home.card`, a `settings.section`, a
 * `details.panel` — whose drawing is a frame (`frame.tsx`), registered under the application's id
 * like an installed module's. Modals open through the shell's dialog when a frame asks.
 *
 * When each is made and gone (the UI design's §3): a space's centre and left column when the space
 * is first entered, its tabs when first selected — kept, hidden, while away (`keeper.tsx`) — and
 * gone when the application stops; a second panel, a settings section, a card, a modal for as long
 * as they are shown.
 *
 * The application's state follows `harnix.apps`: one that stops loses its frames, and an unsigned
 * one under `[apps] dev = true` whose `ui/` changed (`runtime.ui_rev`) has its frames reloaded.
 */

import { createRoot } from "react-dom/client";
import type { Hello } from "@harnix/core";
import type { ReactNode } from "react";
import type { SpaceSecondary, SpaceTab } from "@harnix/react";

import type { App } from "../app.ts";
import { Body } from "../dialogs.tsx";
import { topicsOf } from "../topics.ts";
import { QuestionLayer } from "./confirm.tsx";
import { SurfaceView, type SurfaceActions } from "./frame.tsx";
import { KeeperLayer, KeptSlot, keeper } from "./keeper.tsx";
import { interfacesOf, titleOf, type AppInterface, type SurfaceDecl } from "./model.ts";
import { surfaces } from "./store.ts";

/** Each space's second panel, while it is open: what its frame was opened with. */
const secondaries = new Map<string, { params: unknown; title?: string }>();
/** Each application space's badge. */
const badges = new Map<string, string | number>();

let layer = false;

/** The page's one layer for what is drawn outside the shell's tree: kept frames, the shell's questions. */
function mountLayer(): void {
  if (layer || typeof document === "undefined") return;
  layer = true;
  const root = document.createElement("div");
  root.id = "hx-surfaces";
  document.body.append(root);
  createRoot(root).render(
    <>
      <KeeperLayer />
      <QuestionLayer />
    </>,
  );
}

/** What a frame of `iface` may have the shell do for it, in the space `space`. */
function actionsFor(app: App, iface: AppInterface, space: string | null): SurfaceActions {
  return {
    space,
    openModal: (id, params) => openModal(app, iface, id, params),
    secondary(open) {
      if (!space) return;
      if (open) secondaries.set(space, open);
      else secondaries.delete(space);
      app.redraw();
    },
    badge(value) {
      if (!space) return;
      if (value === null || value === 0 || value === "") badges.delete(space);
      else badges.set(space, value);
      app.redraw();
    },
  };
}

/** One of the application's modals, in the shell's dialog; what it closed with (null: dismissed). */
function openModal(app: App, iface: AppInterface, id: string, params: unknown): Promise<unknown> {
  const decl = iface.surfaces.find((s) => s.point === "modal" && s.id === id);
  if (!decl) return Promise.resolve(null);
  return new Promise((resolve) => {
    let settled = false;
    const settle = (value: unknown) => {
      if (settled) return;
      settled = true;
      resolve(value);
    };
    const actions = actionsFor(app, iface, firstSpace(iface));
    app.openModal({
      title: titleOf(iface, decl),
      wide: decl.wide,
      content: (
        <Body>
          <Gone onGone={() => settle(null)}>
            <SurfaceView
              app={app}
              iface={iface}
              decl={decl}
              actions={actions}
              params={params}
              onClose={(result) => {
                settle(result);
                app.closeModal();
              }}
            />
          </Gone>
        </Body>
      ),
    });
  });
}

/** Calls `onGone` when it leaves the page: a dialog closed by the shell (Escape, its ×, another). */
function Gone({ onGone, children }: { onGone: () => void; children: ReactNode }) {
  return (
    <div
      className="hx-modal-surface"
      ref={(el) => {
        if (!el) onGone();
      }}
    >
      {children}
    </div>
  );
}

function firstSpace(iface: AppInterface): string | null {
  return iface.surfaces.find((s) => s.point === "space")?.space ?? null;
}

/** Register one application's surfaces. */
function register(app: App, iface: AppInterface): void {
  const owner = iface.id;
  const reg = app.registry;
  const of = (point: SurfaceDecl["point"], space?: string | null) => iface.surfaces.filter((s) => s.point === point && (space === undefined || s.space === space));

  for (const centre of of("space")) {
    const space = centre.space!;
    const actions = actionsFor(app, iface, space);
    const sidebar = of("space.sidebar", space)[0];
    const secondary = of("space.secondary", space)[0];
    const tabs: SpaceTab[] = of("space.tab", space).map((tab) => ({
      id: tab.id,
      title: () => titleOf(iface, tab),
      render: () => <KeptSlot app={app} iface={iface} decl={tab} actions={actions} />,
    }));
    const center = centre.center ?? "hidden";
    reg.register(
      "space",
      {
        id: space,
        icon: centre.icon ?? "file",
        order: centre.order ?? 60,
        title: () => titleOf(iface, centre),
        center,
        ...(centre.sessions ? { sessions: centre.sessions } : { newSession: false }),
        ...(center !== "chat" && centre.frame ? { overview: () => <KeptSlot app={app} iface={iface} decl={centre} actions={actions} /> } : {}),
        ...(sidebar ? { sidebar: () => <KeptSlot app={app} iface={iface} decl={sidebar} actions={actions} /> } : {}),
        ...(tabs.length ? { tabs } : {}),
        ...(secondary
          ? {
              secondary: (): SpaceSecondary | null => {
                const open = secondaries.get(space);
                if (!open) return null;
                return {
                  key: `${owner}:secondary`,
                  title: () => open.title ?? titleOf(iface, secondary),
                  render: () => <SurfaceView app={app} iface={iface} decl={secondary} actions={actions} params={open.params} />,
                  onClose: () => {
                    secondaries.delete(space);
                    app.redraw();
                  },
                };
              },
            }
          : {}),
        badge: () => badges.get(space) ?? null,
      },
      owner,
    );
  }

  const elsewhere = actionsFor(app, iface, firstSpace(iface));
  for (const card of of("home.card")) {
    reg.register(
      "home.card",
      {
        id: `${owner}.${card.id}`,
        order: card.order ?? 50,
        ...(card.title ? { title: () => titleOf(iface, card) } : {}),
        render: () => <SurfaceView app={app} iface={iface} decl={card} actions={elsewhere} />,
      },
      owner,
    );
  }
  for (const section of of("settings.section")) {
    reg.register(
      "settings.section",
      {
        id: `${owner}.${section.id}`,
        label: () => titleOf(iface, section),
        render: () => <SurfaceView app={app} iface={iface} decl={section} actions={elsewhere} />,
      },
      owner,
    );
  }
  for (const panel of of("details.panel")) {
    reg.register(
      "details.panel",
      {
        id: `${owner}.${panel.id}`,
        match: (item) => panel.kinds.includes(item.kind),
        title: () => titleOf(iface, panel),
        render: (item) => <SurfaceView app={app} iface={iface} decl={panel} actions={elsewhere} params={item} />,
      },
      owner,
    );
  }
}

interface Listed {
  apps?: { id: string; enabled?: boolean; runtime?: { state?: string; ui_rev?: string | null } }[];
}

/** Follow the applications' state: frames go with a stopped application, reload with a changed `ui/`. */
function follow(app: App, ids: Set<string>): void {
  if (!app.snapshot.reachable.includes("harnix.apps.list")) return;
  const revs = new Map<string, string>();
  topicsOf(app.client).subscribe("harnix.apps", {}, (value) => {
    for (const row of (value as Listed | null)?.apps ?? []) {
      if (!ids.has(row.id)) continue;
      const running = Boolean(row.enabled) && row.runtime?.state === "running";
      surfaces.setStopped(row.id, !running);
      if (!running) keeper.drop(row.id);
      const rev = row.runtime?.ui_rev;
      if (!rev) continue;
      const was = revs.get(row.id);
      revs.set(row.id, rev);
      // Every frame of it, made again from its files.
      if (was !== undefined && was !== rev) surfaces.bump(row.id);
    }
  });
}

/** Put the isolated applications on the page; hello's contributions say which they are. */
export async function loadSurfaces(app: App, hello: Hello): Promise<void> {
  const { apps, refused } = interfacesOf(hello);
  for (const r of refused) app.registry.note({ point: r.point, owner: r.app, id: r.module, reason: r.reason });
  if (!apps.length) return;
  mountLayer();
  for (const iface of apps) {
    try {
      register(app, iface);
    } catch (e) {
      app.registry.note({ point: "surface", owner: iface.id, reason: e instanceof Error ? e.message : String(e) });
    }
  }
  follow(app, new Set(apps.map((a) => a.id)));
}
