/**
 * @harnix/schedule — the `schedule` extension's interface (harnix-app.md §9): schedules made, seen
 * and run from where the thing they do lives, through other packages' points only.
 *
 * - The session menu's "定时…" (`session.action`): the session's schedules and a new one, a turn in it.
 * - "定时运行这个流程…" in the new-task dialog and a flow's detail (`flow.start.action`, `@harnix/flow`'s):
 *   the editor with a task target, what was filled in carried over.
 * - A project's overview, block 40 (`harnix-app.project.panels`): its schedules, a new session or task.
 * - Clocks: on a session's row (`harnix-app.session.badge`), a task's next time on its row (`flow.task.badge`).
 * - Every schedule at once, its source, state and next time, and the models' jobs, read-only and
 *   deletable (`AllSchedules`): a block of the home page (`harnix-app.home.panels`, 50).
 *
 * A point nobody defines (no projects space, no flows) is dropped quietly. The data is the topic
 * `schedule.list`, followed while drawn, and the `schedule.*` methods.
 */

import { t, type ActionTarget, type Dispose, type HostApi, type Registry } from "@harnix/react";

import { SessionClock, TaskNext } from "./badges";
import { scheduleEditor } from "./editor";
import { ScheduleStore } from "./store";
import { registerScheduleStrings } from "./strings";
import { AllSchedules, ProjectSchedules, sessionSchedulesDialog, type PanelContext } from "./views";
import "./schedule.css";

export { scheduleEditor, ScheduleEditor, type EditorContext } from "./editor";
export { FREQUENCY_KINDS, WORKDAYS, cronShape, defaultFrequency, describeTiming, frequencyProblems, fromTiming, monthlyOf, toTiming, weekdaysText, type Frequency, type FrequencyKind } from "./frequency";
export { changedFields, changesAnything, createParams, draftProblems, initialDraft, targetOf, updateParams, type Draft, type TargetKind, type TaskSeed } from "./draft";
export { allZones, formatWhen, isZone, localZone, momentOf, offsetLabel, offsetMinutes, relativeTo, wallClockOf } from "./time";
export { ScheduleStore, activeOfSession, ofProject, ofSession, scheduleOfRun, stateOf, type ScheduleState } from "./store";
export { STRINGS, STRINGS_OWNER } from "./strings";
export { AllSchedules } from "./views";

/** The backend extension this is the interface of. */
export const extension = "schedule";

/** The points of other packages this one fills; each dropped quietly where nobody defines it. */
const SESSION_BADGE = "harnix-app.session.badge";
const PROJECT_PANELS = "harnix-app.project.panels";
const HOME_PANELS = "harnix-app.home.panels";
const TASK_BADGE = "flow.task.badge";
const START_ACTION = "flow.start.action";

/** The clock, drawn on the shell's 24 grid: `schedule.clock`. */
const ICONS = { clock: `<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/>` };

/** A flow as `flow.start.action` hands it over. */
interface StartDraft {
  flow: string;
  subject?: string;
  inputs?: Record<string, unknown>;
  title?: string;
  output_dir?: string;
  workspace?: string;
  project_root?: string;
  shown_in?: string;
}

export function register(r: Registry, host: HostApi): Dispose {
  registerScheduleStrings(r);
  r.icons(ICONS, extension);
  const store = new ScheduleStore(host);
  const quiet = { orElse: () => undefined };

  // The session a menu was opened for: the space it belongs to (as the hub lists it), its name; a
  // session another extension owns and only shows here (a task's) is that extension's to schedule.
  const sessionAt = (target: ActionTarget) => {
    const ref = target.session;
    let row: { space?: string; name?: string | null; exposed_from?: string | null } | undefined;
    try {
      row = host.session.list().find((s) => s.session_id === ref.sessionId && (s.instance ?? host.defaults.instance) === ref.instance) as typeof row;
    } catch {
      row = undefined;
    }
    const claimed = r.at("space").find((c) => c.id === target.space)?.sessions ?? target.space;
    return { row, space: row?.space ?? claimed };
  };
  r.register(
    "session.action",
    {
      id: "schedule.session",
      at: ["menu"],
      icon: "schedule.clock",
      label: () => t("schedule.action.session"),
      match: (target) => target.kind === "session" && !sessionAt(target).row?.exposed_from,
      run: (target) => {
        const { row, space } = sessionAt(target);
        void sessionSchedulesDialog(host, r, store, { instance: target.session.instance, session_id: target.session.sessionId, space, name: row?.name ?? null });
      },
    },
    extension,
  );

  r.register(
    SESSION_BADGE,
    { id: "schedule.clock", render: (session: { instance: string; sessionId: string }) => <SessionClock store={store} instance={session.instance} sessionId={session.sessionId} /> },
    extension,
    quiet,
  );
  r.register(TASK_BADGE, { id: "schedule.next", render: (run: { trigger?: { kind: string; schedule?: string } | null }) => <TaskNext store={store} run={run} /> }, extension, quiet);
  r.register(
    START_ACTION,
    {
      id: "schedule.task",
      icon: "schedule.clock",
      label: () => t("schedule.action.task"),
      // From a flow's detail there is no project yet (`null`): the editor asks for one.
      run: (draft: StartDraft) =>
        scheduleEditor(host, r, { kinds: ["task"], task: draft, project: draft.project_root ?? null, space: draft.shown_in, shownIn: draft.shown_in }).then(() => undefined),
    },
    extension,
    quiet,
  );
  r.register(
    PROJECT_PANELS,
    { id: "schedule.project", order: 40, title: () => t("schedule.title"), render: (context: PanelContext) => <ProjectSchedules host={host} r={r} store={store} context={context} /> },
    extension,
    quiet,
  );
  // Every schedule, the models' jobs with them, across the home page under the lists.
  r.register(
    HOME_PANELS,
    { id: "schedule.all", order: 50, wide: true, title: () => t("schedule.title"), render: () => <AllSchedules host={host} r={r} store={store} /> },
    extension,
    quiet,
  );
  return () => store.stop();
}
