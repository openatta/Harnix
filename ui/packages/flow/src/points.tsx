/**
 * The two points this package defines for other packages to fill (harnix-ext.md, the points first-party
 * extensions define; the flow extension declares the same two):
 *
 *   flow.task.badge    a badge on a task's row, beside the clock of one a schedule started: another
 *                      package's word about the run (a schedule's next time, say). `render` point.
 *   flow.start.action  another way to start a flow than now, offered by the new-task dialog (with
 *                      what was filled in) and by a flow's detail (the flow alone): "schedule it".
 *                      `register` point.
 *
 * This package knows neither by name: it draws whatever is registered. Nothing registered, nothing drawn.
 */

import type { ReactNode } from "react";
import { Contributed, Glyph, useContributions, type Registry } from "@harnix/react";

import type { RunSummary } from "./types";

export const TASK_BADGE = "flow.task.badge";
export const START_ACTION = "flow.start.action";

/** A badge on a task's row (`flow.task.badge`). */
export interface TaskBadge {
  id: string;
  /** Inside the row's badges; nothing to say, nothing drawn (null). */
  render(run: RunSummary): ReactNode;
}

/** What a flow would be started with, as far as it is known where the action is offered. */
export interface StartDraft {
  flow: string;
  subject?: string;
  inputs?: Record<string, unknown>;
  title?: string;
  output_dir?: string;
  workspace?: "worktree" | "project";
  /** The project, when the dialog is a project's. */
  project_root?: string;
  /** The space a task's main session would be shown in (`flow.start`'s `shown_in`). */
  shown_in?: string;
}

/** Another way to start a flow (`flow.start.action`). */
export interface StartAction {
  id: string;
  label(): string;
  icon?: string;
  /** Called once the place that offered it has let go (the dialog is closed). */
  run(draft: StartDraft): void | Promise<void>;
}

/** A run's badges from the other packages, each in its own guard. */
export function TaskBadges({ registry, run }: { registry: Registry; run: RunSummary }) {
  const badges = useContributions<TaskBadge>(TASK_BADGE, registry);
  return (
    <>
      {badges.map((badge) => (
        <Contributed key={`${badge.owner}/${badge.id}`} registry={registry} point={TASK_BADGE} contribution={badge}>
          <BadgeBody badge={badge} run={run} />
        </Contributed>
      ))}
    </>
  );
}

/** Called inside the guard: a `render` that throws costs that badge, not the row. */
function BadgeBody({ badge, run }: { badge: TaskBadge; run: RunSummary }) {
  return <>{badge.render(run)}</>;
}

/**
 * The other ways to start, as buttons: `before` is called first (the dialog closes), then the action
 * with the draft. A throw is the action's failure, counted against it.
 */
export function StartActions({ registry, draft, disabled, before, testid }: { registry: Registry; draft: () => StartDraft | null; disabled?: boolean; before?: () => void; testid: string }) {
  const actions = useContributions<StartAction>(START_ACTION, registry);
  return (
    <>
      {actions.map((action) => {
        let label = action.id;
        try {
          label = action.label();
        } catch {
          // A label that fails is the id.
        }
        return (
          <button
            type="button"
            key={`${action.owner}/${action.id}`}
            className="btn outline sm"
            data-testid={`${testid}-${action.id}`}
            disabled={disabled}
            onClick={() => {
              const value = draft();
              if (!value) return;
              before?.();
              void Promise.resolve()
                .then(() => action.run(value))
                .catch((e: unknown) => registry.fail(START_ACTION, action, e));
            }}
          >
            {action.icon ? <Glyph name={action.icon} className="glyph" /> : null}
            {label}
          </button>
        );
      })}
    </>
  );
}
