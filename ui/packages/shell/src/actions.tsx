/**
 * `session.action`: what the shell asks at the title bar, the session menu and a tool row, and how
 * it runs one. The shell's own menu items (open, fork, close, delete, rename) use the kernel's
 * methods and stay the shell's; what belongs to one extension — opening a session in the system
 * space, say — is a registration here, drawn wherever its `at` says.
 */

import { t, useRegistry, type ActionPlace, type ActionTarget, type SessionAction, Glyph } from "@harnix/react";

import type { App } from "./app.ts";
import { hostDialogs } from "./host-dialogs.tsx";
import type { MenuEntry } from "./Menu.tsx";

type Registered = SessionAction & { owner: string };

/** What one action is called for this target; a label that throws drops it, counted as failed. */
function labelOf(app: App, action: Registered, target: ActionTarget): string | null {
  try {
    return action.label(target);
  } catch (e) {
    queueMicrotask(() => app.registry.fail("session.action", action, e));
    return null;
  }
}

/** Run one: a `danger` one is confirmed first; a failure is the shell's error line. */
export async function runAction(app: App, action: Registered, target: ActionTarget, label: string): Promise<void> {
  if (action.danger && !(await hostDialogs(app).confirm(label, { danger: true, confirm: label }))) return;
  try {
    await action.run(target);
  } catch (e) {
    app.notice(t("action.failed", { action: label, error: e instanceof Error ? e.message : String(e) }), "error");
  }
}

/** The actions at a place for a target, with their labels. */
function asked(app: App, place: ActionPlace, target: ActionTarget): { action: Registered; label: string }[] {
  return app.registry.actions(place, target).flatMap((action) => {
    const label = labelOf(app, action, target);
    return label === null ? [] : [{ action, label }];
  });
}

/** The session menu's items for a target, after the shell's own. */
export function actionMenuItems(app: App, target: ActionTarget): MenuEntry[] {
  return asked(app, "menu", target).map(({ action, label }) => ({
    id: `action:${action.id}`,
    icon: action.icon,
    label,
    danger: action.danger,
    testId: `action-${action.id}`,
    run: () => void runAction(app, action, target, label),
  }));
}

/** The actions of a place, as buttons: icon ones at the title bar, text ones under a tool call. */
export function ActionButtons({ app, place, target }: { app: App; place: "header" | "tool"; target: ActionTarget }) {
  useRegistry(app.registry);
  return (
    <>
      {asked(app, place, target).map(({ action, label }) =>
        place === "header" ? (
          <button
            key={action.id}
            type="button"
            className="ib"
            data-testid={`action-${action.id}`}
            title={label}
            aria-label={label}
            onClick={() => void runAction(app, action, target, label)}
          >
            <Glyph name={action.icon ?? "more"} className="glyph" />
          </button>
        ) : (
          <button key={action.id} type="button" className="btn sm outline" data-testid={`action-${action.id}`} onClick={() => void runAction(app, action, target, label)}>
            {label}
          </button>
        ),
      )}
    </>
  );
}
