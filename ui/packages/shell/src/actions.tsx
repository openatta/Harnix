/**
 * `session.action`: what the shell asks at the title bar, the session menu, a tool row and a
 * person's message (its menu), and how it runs one. The shell's own menu items (open, fork, close, delete, rename) use the kernel's
 * methods and stay the shell's; what belongs to one extension — opening a session in the system
 * space, say — is a registration here, drawn wherever its `at` says.
 */

import { t, useRegistry, type ActionPlace, type ActionTarget, type SessionAction, Glyph } from "@harnix/react";

import type { App } from "./app.ts";
import { hostDialogs } from "./host-dialogs.tsx";
import { menuBelow, openMenu, type MenuEntry } from "./Menu.tsx";

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

/** Why one is not for now, if it says so; a throw counts as enabled. */
function disabledOf(app: App, action: Registered, target: ActionTarget): string | null {
  try {
    return action.disabled?.(target) || null;
  } catch (e) {
    queueMicrotask(() => app.registry.fail("session.action", action, e));
    return null;
  }
}

/** The actions at a place for a target, with their labels and why each is not for now. */
function asked(app: App, place: ActionPlace, target: ActionTarget): { action: Registered; label: string; disabled: string | null }[] {
  return app.registry.actions(place, target).flatMap((action) => {
    const label = labelOf(app, action, target);
    return label === null ? [] : [{ action, label, disabled: disabledOf(app, action, target) }];
  });
}

/** The session menu's items for a target, after the shell's own. */
export function actionMenuItems(app: App, target: ActionTarget): MenuEntry[] {
  return entries(app, "menu", target);
}

function entries(app: App, place: ActionPlace, target: ActionTarget): MenuEntry[] {
  return asked(app, place, target).map(({ action, label, disabled }) => ({
    id: `action:${action.id}`,
    icon: action.icon,
    label,
    sub: disabled ?? undefined,
    danger: action.danger,
    disabled: disabled !== null,
    testId: `action-${action.id}`,
    run: () => void runAction(app, action, target, label),
  }));
}

/**
 * A person's message's menu: `⋯` beside it, when an action at `message` matches it (going back to
 * it, say). Under the bubble, shown on hover or focus as the copy button under an answer is.
 */
export function MessageMenu({ app, target }: { app: App; target: ActionTarget }) {
  useRegistry(app.registry);
  if (!app.registry.actions("message", target).length) return null;
  return (
    <button
      type="button"
      data-testid="conv-message-menu"
      title={t("flow.messageMenu")}
      aria-label={t("flow.messageMenu")}
      aria-haspopup="menu"
      onClick={(e) =>
        openMenu({
          label: t("flow.messageMenu"),
          at: menuBelow(e.currentTarget),
          returnTo: e.currentTarget,
          testId: "message-menu",
          items: entries(app, "message", target),
        })
      }
    >
      <Glyph name="more" className="glyph" />
    </button>
  );
}

/**
 * A tool row's argument: a link when an action at `arg` matches the call — the first one, in
 * registration order, its label the link's title — and plain text otherwise. A click runs it and
 * leaves the row as it was (open or closed). The argument sits in the row's toggle, a button, so it
 * is not a second control for the keyboard: the same action is a button under the call when it is
 * also `at: ["tool"]`.
 */
export function ActionArg({ app, target, className, testId, children }: { app: App; target: ActionTarget; className: string; testId: string; children: string }) {
  useRegistry(app.registry);
  const first = asked(app, "arg", target)[0];
  if (!first) {
    return (
      <span className={className} data-testid={testId}>
        {children}
      </span>
    );
  }
  return (
    <span
      className={`${className} arg-link`}
      data-testid={testId}
      data-action={first.action.id}
      title={first.label}
      onClick={(e) => {
        e.stopPropagation();
        e.preventDefault();
        void runAction(app, first.action, target, first.label);
      }}
    >
      {children}
    </span>
  );
}

/** The actions of a place, as buttons: icon ones at the title bar, text ones under a tool call. */
export function ActionButtons({ app, place, target }: { app: App; place: "header" | "tool"; target: ActionTarget }) {
  useRegistry(app.registry);
  return (
    <>
      {asked(app, place, target).map(({ action, label, disabled }) =>
        place === "header" ? (
          <button
            key={action.id}
            type="button"
            className="ib"
            data-testid={`action-${action.id}`}
            title={disabled ?? label}
            aria-label={label}
            disabled={disabled !== null}
            onClick={() => void runAction(app, action, target, label)}
          >
            <Glyph name={action.icon ?? "more"} className="glyph" />
          </button>
        ) : (
          <button
            key={action.id}
            type="button"
            className="btn sm outline"
            data-testid={`action-${action.id}`}
            title={disabled ?? undefined}
            disabled={disabled !== null}
            onClick={() => void runAction(app, action, target, label)}
          >
            {label}
          </button>
        ),
      )}
    </>
  );
}
