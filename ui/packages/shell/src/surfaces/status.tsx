/**
 * The system space's Apps tab, for an application drawn in frames: this page's surfaces of it —
 * how many are live, how often they failed, each one's state — and the bridge requests it was
 * refused. Page-local (`store.ts`): the backend cannot see a frame, and each window has its own.
 */

import { useSyncExternalStore } from "react";
import { useT } from "@harnix/react";

import { surfaces } from "./store.ts";

/** `live · failures`, or a dash for an application with no frame on this page. */
export function SurfaceCount({ app }: { app: string }) {
  useSyncExternalStore(surfaces.subscribe, surfaces.getVersion);
  const s = surfaces.summary(app);
  return <>{s.surfaces.length || s.crashes ? `${s.live} · ${s.crashes}` : "—"}</>;
}

export function SurfaceDetail({ app }: { app: string }) {
  const t = useT();
  useSyncExternalStore(surfaces.subscribe, surfaces.getVersion);
  const s = surfaces.summary(app);
  return (
    <div data-testid="system-app-surfaces">
      <div className="turnh">{t("system.apps.surfaces")}</div>
      {s.surfaces.length ? (
        <ul>
          {s.surfaces.map((x) => (
            <li key={x.key} data-testid="system-app-surface" data-point={x.point} data-state={x.state}>
              <code>{x.point}</code> {x.id} — {t(`system.apps.surface.${x.state}`)}
              {x.reason ? <span className="muted"> ({x.reason})</span> : null}
            </li>
          ))}
        </ul>
      ) : (
        <div className="muted">{t("system.apps.noSurfaces")}</div>
      )}
      <div className="turnh">{t("system.apps.bridgeRefused")}</div>
      {s.refused.length ? (
        <ul>
          {s.refused.map((r, i) => (
            <li key={i} data-testid="system-app-bridge-refusal">
              <code>{r.name}</code> — {r.reason}
            </li>
          ))}
        </ul>
      ) : (
        <div className="muted">{t("system.apps.noBridgeRefusals")}</div>
      )}
    </div>
  );
}
