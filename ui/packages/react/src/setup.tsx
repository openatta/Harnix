/**
 * A first-run step, as a `home.card` draws it: what is missing and the way to it, until it is done
 * or closed. Each package that owns a step registers its own card (the model is the settings', a
 * project the Harnix app's) and says from its own data whether the step is done; the look, and
 * which steps were closed, are shared here.
 *
 * A step done before the page first saw it open is not drawn at all; one finished while the page
 * is open shows as done (a check) until it is closed or the page goes. Closed steps are kept in
 * this browser (`harnix.setup.dismissed`). A card that draws nothing takes no place on the empty
 * page (the shell hides a `home.card` with no content).
 */

import { useSyncExternalStore, type ReactNode } from "react";

import { Glyph } from "./glyph";
import { useT } from "./i18n/index";

const DISMISSED_KEY = "harnix.setup.dismissed";

/** How a step is drawn: not at all, still to do, or done while this page watched. */
export type SetupStepState = "hidden" | "todo" | "done";

/** The steps this page has seen still to do: one of them done since shows as done. */
const seenOpen = new Set<string>();
/** Closed on this page: what holds when the browser keeps nothing (private mode). */
const closedHere = new Set<string>();
const listeners = new Set<() => void>();
let version = 0;

function readDismissed(): string[] {
  try {
    const raw = localStorage.getItem(DISMISSED_KEY);
    const ids = raw == null ? [] : (JSON.parse(raw) as unknown);
    return Array.isArray(ids) ? ids.filter((id): id is string => typeof id === "string") : [];
  } catch {
    return [];
  }
}

/** Close a step for good, in this browser. */
export function dismissSetupStep(id: string): void {
  closedHere.add(id);
  const ids = readDismissed();
  if (!ids.includes(id)) {
    try {
      localStorage.setItem(DISMISSED_KEY, JSON.stringify([...ids, id]));
    } catch {
      /* private mode: closed for this page only */
    }
  }
  version += 1;
  for (const fn of listeners) fn();
}

/** Whether a step was closed: a step that was need not read whether it is done. */
export function setupStepDismissed(id: string): boolean {
  return closedHere.has(id) || readDismissed().includes(id);
}

/**
 * How a step stands, from whether it is done: `null` while that is not known yet (nothing drawn,
 * so a step already done does not flash up while it is read).
 */
export function setupStepState(id: string, done: boolean | null): SetupStepState {
  if (done === null || setupStepDismissed(id)) return "hidden";
  if (!done) {
    seenOpen.add(id);
    return "todo";
  }
  return seenOpen.has(id) ? "done" : "hidden";
}

const subscribe = (fn: () => void) => {
  listeners.add(fn);
  return () => listeners.delete(fn);
};

/** A step's state, drawn again when a step is closed. */
export function useSetupStep(id: string, done: boolean | null): SetupStepState {
  useSyncExternalStore(subscribe, () => version);
  return setupStepState(id, done);
}

/** One step's card body: its title (a check once done), what it is about, its actions, a close button. */
export function SetupStep({
  id,
  state,
  icon,
  title,
  children,
  actions,
}: {
  id: string;
  state: SetupStepState;
  /** The glyph before the title while the step is still to do. */
  icon: string;
  title: ReactNode;
  children?: ReactNode;
  actions?: ReactNode;
}) {
  const t = useT();
  if (state === "hidden") return null;
  return (
    <div className="setup-step" data-testid="setup-step" data-step={id} data-state={state}>
      <div className="dt">
        <Glyph name={state === "done" ? "check" : icon} className="glyph" />
        <span className="truncate">{title}</span>
        <span className="sp" />
        <button
          type="button"
          className="ib sm"
          data-testid="setup-dismiss"
          title={t("setup.dismiss")}
          aria-label={t("setup.dismiss")}
          onClick={() => dismissSetupStep(id)}
        >
          <Glyph name="close" className="glyph" />
        </button>
      </div>
      {children ? <div className="setup-body">{children}</div> : null}
      {actions ? <div className="setup-actions">{actions}</div> : null}
    </div>
  );
}

/** For tests: forget what this page saw. */
export function resetSetupSteps(): void {
  seenOpen.clear();
  closedHere.clear();
  version += 1;
}
