/**
 * The settings panel's pieces, for a `settings.section` to draw with: a labelled row, a value, a
 * titled block whose content is loaded, and the panel's reload. The panel is the shell's; a
 * section from any UI package looks like the shell's own because it is drawn with these.
 */

import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { errorText } from "@harnix/core";

import { useT } from "./i18n/index";

/** Bumped after a write, so every section on the panel reads again. */
const Reload = createContext<{ version: number; reload: () => void }>({ version: 0, reload: () => {} });

/** The panel's reload, around the page it draws (the shell's settings panel). */
export function SettingsReload({ version, reload, children }: { version: number; reload: () => void; children: ReactNode }) {
  return <Reload.Provider value={{ version, reload }}>{children}</Reload.Provider>;
}

/** Every section on the page reads again (after a write). */
export function useSettingsReload(): () => void {
  return useContext(Reload).reload;
}

export function SettingsRow({ label, hint, children }: { label: ReactNode; hint?: ReactNode; children?: ReactNode }) {
  return (
    <div className="srow-setting">
      <div className="text">
        <div className="label">{label}</div>
        {hint ? <div className="hint">{hint}</div> : null}
      </div>
      <div className="control">{children}</div>
    </div>
  );
}

export function SettingsValue({ tone, children }: { tone?: "ok" | "warn" | "err"; children: ReactNode }) {
  return <span className={`value${tone ? ` ${tone}` : ""}`}>{children}</span>;
}

/** A titled block whose content is loaded; a late answer for a page no longer shown is dropped. */
export function SettingsSection<T>({ title, load, children }: { title: string; load: () => Promise<T>; children: (data: T) => ReactNode }) {
  const t = useT();
  const { version } = useContext(Reload);
  const [state, setState] = useState<{ data?: T; error?: string } | null>(null);
  useEffect(() => {
    let alive = true;
    setState(null);
    load().then(
      (data) => alive && setState({ data }),
      (e: unknown) => alive && setState({ error: errorText(e) }),
    );
    return () => {
      alive = false;
    };
  }, [version]);
  return (
    <>
      <h3>{title}</h3>
      <div>
        {!state ? (
          <div className="hint">{t("common.loading")}</div>
        ) : state.error !== undefined ? (
          <div className="hint err" data-testid="settings-failed">
            {state.error}
          </div>
        ) : (
          children(state.data as T)
        )}
      </div>
    </>
  );
}

/** A dialog's content (`host.dialogs.open`): its body, then its buttons. */
export function DialogBody({ children }: { children: ReactNode }) {
  return <div className="body">{children}</div>;
}

export function DialogFoot({ children }: { children?: ReactNode }) {
  return <div className="foot">{children}</div>;
}
