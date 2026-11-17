/**
 * The editor's draft: what the dialog holds while a schedule is made or changed, what is wrong with
 * it, and what it is sent as — `schedule.create`, or `schedule.update` with only what changed against
 * the revision it was read at (harnix-app.md §9). Pure, so the rules are tested without a page.
 */

import type { Concurrency, CreateParams, Missed, NewMode, Schedule, ScheduleTiming, Target, UpdateParams } from "@harnix/core";

import { defaultFrequency, fromTiming, frequencyProblems, toTiming, type Frequency } from "./frequency";
import { readInputs, type InputField } from "./inputs";
import { isZone, localZone } from "./time";

export type TargetKind = Target["kind"];
export const TARGET_KINDS: TargetKind[] = ["session", "new_session", "task"];

/** At most this long, a title (as the scheduler takes it). */
export const TITLE_MAX = 120;

/** A flow as another package would start it: `flow.start.action`'s draft. */
export interface TaskSeed {
  flow: string;
  subject?: string;
  inputs?: Record<string, unknown>;
  title?: string;
  output_dir?: string;
  workspace?: string;
  project_root?: string;
  show_in_space?: string;
}

/** Where the editor was opened from, and so what it offers. */
export interface EditorContext {
  /** Changing this one; absent: making one. */
  existing?: Schedule;
  /** The space a new one belongs to (the backend space of where it was opened); absent: the default. */
  space?: string;
  /** The targets on offer for a new one, the first picked; an edit keeps its own. */
  kinds: TargetKind[];
  /** The session a session target turns to. */
  session?: { instance: string; session_id: string; name?: string | null };
  /** The project, when the place it was opened from is one's: not asked. */
  project?: string | null;
  /** A flow to schedule, as the new-task dialog had it filled in. */
  task?: TaskSeed;
  /** The space a task's main session is shown in (the one the opener lists sessions of). */
  showInSpace?: string;
}

export interface Draft {
  title: string;
  freq: Frequency;
  zone: string;
  kind: TargetKind;
  /** A session or new-session target's turn. */
  prompt: string;
  mode: NewMode;
  /** new_session and task: the project directory. */
  project: string;
  scene: string;
  model: string;
  flow: string;
  subject: string;
  /** The inputs' boxes, by key. */
  inputs: Record<string, string>;
  taskTitle: string;
  outputDir: string;
  /** "": as the flow says. */
  workspace: "" | "worktree" | "project";
  missed: Missed;
  concurrency: Concurrency;
}

const workspaceOf = (w: string | undefined): Draft["workspace"] => (w === "worktree" || w === "project" ? w : "");

/** The draft a dialog opens with: the schedule being changed, else what the opener knows. */
export function initialDraft(ctx: EditorContext, now = Date.now()): Draft {
  const zone = ctx.existing?.time_zone ?? localZone();
  const base: Draft = {
    title: "",
    freq: defaultFrequency(now, zone),
    zone,
    kind: ctx.kinds[0] ?? "session",
    prompt: "",
    mode: "new_once_then_reuse",
    project: ctx.project ?? ctx.task?.project_root ?? "",
    scene: "",
    model: "",
    flow: ctx.task?.flow ?? "",
    subject: ctx.task?.subject ?? "",
    inputs: {},
    taskTitle: ctx.task?.title ?? "",
    outputDir: ctx.task?.output_dir ?? "",
    workspace: workspaceOf(ctx.task?.workspace),
    missed: "catch_up_latest",
    concurrency: "skip",
  };
  const s = ctx.existing;
  if (!s) return base;
  const draft: Draft = { ...base, title: s.title, freq: fromTiming(s.timing, zone, base.freq), kind: s.target.kind, missed: s.missed, concurrency: s.concurrency };
  const target = s.target;
  if (target.kind === "session") return { ...draft, prompt: target.prompt };
  if (target.kind === "new_session") {
    return { ...draft, prompt: target.prompt, mode: target.mode, project: target.project_root ?? "", scene: target.scene ?? "", model: target.model ?? "" };
  }
  return {
    ...draft,
    flow: target.flow,
    subject: target.subject,
    project: target.project_root,
    taskTitle: target.title ?? "",
    outputDir: target.output_dir ?? "",
    workspace: workspaceOf(target.workspace),
  };
}

/** The inputs a task target had, for the boxes once the flow's fields are known. */
export function seedInputs(ctx: EditorContext): Record<string, unknown> | undefined {
  const t = ctx.existing?.target;
  if (t?.kind === "task") return t.inputs;
  return ctx.task?.inputs;
}

/** The session a session target turns to: the schedule's, else the opener's. */
function sessionOf(ctx: EditorContext): { instance: string; session_id: string } | null {
  const t = ctx.existing?.target;
  if (t?.kind === "session") return { instance: t.instance, session_id: t.session_id };
  return ctx.session ? { instance: ctx.session.instance, session_id: ctx.session.session_id } : null;
}

const absolute = (p: string) => p.trim().startsWith("/");

/**
 * What is wrong with a draft, by field (a frequency's own fields, `title`, `zone`, `prompt`,
 * `project`, `flow`, `subject`, `input.<key>`, `session`), each as a key of this package's words;
 * none, an empty object.
 */
export function draftProblems(draft: Draft, ctx: EditorContext, fields: InputField[] = [], now = Date.now()): Record<string, string> {
  const out: Record<string, string> = {};
  const title = draft.title.trim();
  if (!title) out.title = "schedule.err.title";
  else if ([...title].length > TITLE_MAX) out.title = "schedule.err.titleLong";
  if (!isZone(draft.zone)) out.zone = "schedule.err.zone";
  else Object.assign(out, frequencyProblems(draft.freq, draft.zone, now));
  if (draft.kind === "session") {
    if (!sessionOf(ctx)) out.session = "schedule.err.session";
    if (!draft.prompt.trim()) out.prompt = "schedule.err.prompt";
  } else if (draft.kind === "new_session") {
    if (!draft.prompt.trim()) out.prompt = "schedule.err.prompt";
    if (draft.project.trim() && !absolute(draft.project)) out.project = "schedule.err.project";
  } else {
    if (!draft.flow.trim()) out.flow = "schedule.err.flow";
    if (!draft.subject.trim()) out.subject = "schedule.err.subject";
    if (!draft.project.trim()) out.project = "schedule.err.projectNeeded";
    else if (!absolute(draft.project)) out.project = "schedule.err.project";
    for (const [key, why] of Object.entries(readInputs(fields, draft.inputs).errors)) out[`input.${key}`] = why;
  }
  return out;
}

const blankless = <T extends Record<string, unknown>>(o: T): T => Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined && v !== "")) as T;

/** The target a draft says. A new session reused so far stays reused while its mode and project do. */
export function targetOf(draft: Draft, ctx: EditorContext, fields: InputField[] = []): Target {
  if (draft.kind === "session") {
    const s = sessionOf(ctx);
    return { kind: "session", instance: s?.instance ?? "", session_id: s?.session_id ?? "", prompt: draft.prompt.trim() };
  }
  if (draft.kind === "new_session") {
    const was = ctx.existing?.target;
    const project = draft.project.trim();
    const keep = was?.kind === "new_session" && was.mode === draft.mode && (was.project_root ?? "") === project ? was.session : undefined;
    return blankless({
      kind: "new_session" as const,
      mode: draft.mode,
      project_root: project || undefined,
      scene: draft.scene.trim() || undefined,
      model: draft.model.trim() || undefined,
      prompt: draft.prompt.trim(),
      session: keep,
    });
  }
  const was = ctx.existing?.target;
  const inputs = readInputs(fields, draft.inputs).inputs;
  return blankless({
    kind: "task" as const,
    flow: draft.flow.trim(),
    subject: draft.subject.trim(),
    inputs: Object.keys(inputs).length ? inputs : undefined,
    project_root: draft.project.trim(),
    title: draft.taskTitle.trim() || undefined,
    output_dir: draft.outputDir.trim() || undefined,
    workspace: draft.workspace || undefined,
    show_in_space: (was?.kind === "task" ? was.show_in_space : undefined) ?? ctx.showInSpace ?? ctx.task?.show_in_space,
  });
}

/** `schedule.create`'s params for a new one. */
export function createParams(draft: Draft, ctx: EditorContext, fields: InputField[] = []): CreateParams {
  const params: CreateParams = {
    title: draft.title.trim(),
    timing: toTiming(draft.freq),
    time_zone: draft.zone,
    target: targetOf(draft, ctx, fields),
    missed: draft.missed,
  };
  if (ctx.space) params.space = ctx.space;
  if (draft.kind === "task") params.concurrency = draft.concurrency;
  return params;
}

/** A timing as it compares: `from` is the scheduler's, never the dialog's. */
function bareTiming(timing: ScheduleTiming): ScheduleTiming {
  if (timing.kind === "after" || timing.kind === "every") {
    const { from: _from, ...rest } = timing;
    return rest as ScheduleTiming;
  }
  return timing;
}

/** Keys in a fixed order, so two objects with the same entries compare equal. */
const canonical = (v: unknown): string =>
  JSON.stringify(v, (_k, value: unknown) =>
    value && typeof value === "object" && !Array.isArray(value) ? Object.fromEntries(Object.entries(value as Record<string, unknown>).sort(([a], [b]) => a.localeCompare(b))) : value,
  );

/**
 * `schedule.update`'s params: only what differs from `base` (the schedule as read, at its revision).
 * An `every` or `after` left as it was is not sent, so it does not count again from now.
 */
export function updateParams(draft: Draft, ctx: EditorContext, base: Schedule, fields: InputField[] = []): UpdateParams {
  const params: UpdateParams = { id: base.id, revision: base.revision };
  const title = draft.title.trim();
  if (title !== base.title) params.title = title;
  const timing = toTiming(draft.freq);
  if (canonical(timing) !== canonical(bareTiming(base.timing))) params.timing = timing;
  if (draft.zone !== base.time_zone) params.time_zone = draft.zone;
  const target = targetOf(draft, { ...ctx, existing: base }, fields);
  if (canonical(target) !== canonical(base.target)) params.target = target;
  if (draft.missed !== base.missed) params.missed = draft.missed;
  if (draft.kind === "task" && draft.concurrency !== base.concurrency) params.concurrency = draft.concurrency;
  return params;
}

/** Whether an update would change anything. */
export function changesAnything(params: UpdateParams): boolean {
  return Object.keys(params).some((k) => k !== "id" && k !== "revision" && k !== "space");
}

/** What another device changed between two readings of a schedule: the names of its fields. */
export function changedFields(before: Schedule, after: Schedule): string[] {
  const fields: (keyof Schedule)[] = ["title", "enabled", "timing", "time_zone", "target", "missed", "concurrency"];
  return fields.filter((f) => canonical(f === "timing" ? bareTiming(before.timing) : before[f]) !== canonical(f === "timing" ? bareTiming(after.timing) : after[f]));
}

/**
 * Keep this draft's changes over a newer revision: each field the draft changed from `before` (the
 * revision it was opened at) stays as typed; each it left alone takes `after`'s, so what the other
 * device changed there is not undone by saving. `opened` is when the draft was made: the defaults a
 * draft fills from the clock are compared as they were then, not as a later day would fill them.
 */
export function rebase(draft: Draft, ctx: EditorContext, before: Schedule, after: Schedule, opened = Date.now()): Draft {
  const was = initialDraft({ ...ctx, existing: before }, opened);
  const now = initialDraft({ ...ctx, existing: after }, opened);
  const out = { ...now } as Record<keyof Draft, unknown>;
  for (const key of Object.keys(draft) as (keyof Draft)[]) {
    if (key === "inputs" || canonical(draft[key]) !== canonical(was[key])) out[key] = draft[key];
  }
  return out as unknown as Draft;
}

