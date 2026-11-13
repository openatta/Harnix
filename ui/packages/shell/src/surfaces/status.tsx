/**
 * In the diagnostics, beside an extension: this page's frames of it — how many are live, how often
 * they failed, each one's state — and the bridge requests it was refused. Page-local
 * (`store.ts`): the backend cannot see a frame, and each window has its own.
 */

import { useSyncExternalStore } from "react";
import { useT } from "@harnix/react";

import { surfaces } from "./store.ts";

/** Nothing for an extension that never had a frame on this page. */
export function PageFrames({ ext }: { ext: string }) {
  const t = useT();
  useSyncExternalStore(surfaces.subscribe, surfaces.getVersion);
  const s = surfaces.summary(ext);
  if (!s.surfaces.length && !s.crashes && !s.refused.length) return null;
  return (
    <div className="page-frames" data-testid="ext-frames" data-ext={ext}>
      <div className="muted">
        {t("diag.frames")} <span data-field="surfaces">{`${s.live} · ${s.crashes}`}</span>
      </div>
      <div className="turnh">{t("diag.frames.surfaces")}</div>
      {s.surfaces.length ? (
        <ul>
          {s.surfaces.map((x) => (
            <li key={x.key} data-testid="ext-surface" data-point={x.point} data-state={x.state}>
              <code>{x.point}</code> {x.id} — {t(`diag.frames.surface.${x.state}`)}
              {x.reason ? <span className="muted"> ({x.reason})</span> : null}
            </li>
          ))}
        </ul>
      ) : (
        <div className="muted">{t("diag.frames.noSurfaces")}</div>
      )}
      <div className="turnh">{t("diag.frames.bridgeRefused")}</div>
      {s.refused.length ? (
        <ul>
          {s.refused.map((r, i) => (
            <li key={i} data-testid="ext-bridge-refusal">
              <code>{r.name}</code> — {r.reason}
            </li>
          ))}
        </ul>
      ) : (
        <div className="muted">{t("diag.frames.noBridgeRefusals")}</div>
      )}
    </div>
  );
}
