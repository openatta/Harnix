/**
 * The schedule editor, a dialog: a title; what it does — a turn in the session it was opened for, a
 * new session (in a project, the first time new then the same one, or new each time; a scene, a
 * model), or a task (a flow, its inputs, a subject, a title, an output directory, a worktree); when,
 * by a guided frequency (`frequency.ts`) in an IANA zone (`zone.tsx`); what a missed time does, and
 * for a task what a previous run still going does. What is wrong is said by its field once a save
 * was tried; what the scheduler refuses (a cron never due) is said as it says it.
 *
 * An edit sends only what changed, with the revision it read. When another device changed it since
 * (`CONFLICT`), the dialog offers to keep this one's changes and review them against the current
 * revision — what the other device changed is named, and taken where this one left a field alone —
 * or to take theirs.
 */

import { useEffect, useMemo, useState } from "react";
import { CONFLICT, type Schedule, type Scene } from "@harnix/core";
import { DialogBody, DialogFoot, t, useHello, useT, type HostApi, type Registry } from "@harnix/react";

import { baseName, errorCode, errorText, sessionName } from "./common";
import { changedFields, changesAnything, createParams, draftProblems, initialDraft, rebase, seedInputs, updateParams, type Draft, type EditorContext, type TargetKind } from "./draft";
import { FREQUENCY_KINDS, describeTiming, toTiming, type Frequency } from "./frequency";
import { inputFields, inputTexts, type InputField } from "./inputs";
import { ZonePicker, rememberZone } from "./zone";

export type { EditorContext } from "./draft";

/** Open the editor; settles with the schedule as saved, or null when it was closed without. */
export function scheduleEditor(host: HostApi, _r: Registry | null, ctx: EditorContext): Promise<Schedule | null> {
  return host.dialogs.open<Schedule>((close) => <ScheduleEditor host={host} ctx={ctx} close={close} />, {
    title: t(ctx.existing ? "schedule.editor.edit" : "schedule.editor.new"),
    wide: true,
  });
}

/** A flow as `flow.defs` lists it, what the editor reads of it. */
interface FlowDef {
  name: string;
  title?: string | null;
  valid?: boolean;
  inputs?: unknown;
}

/** The flows on offer for a project, read again when the project changes. */
function useFlows(host: HostApi, project: string, wanted: boolean): FlowDef[] {
  const [flows, setFlows] = useState<FlowDef[]>([]);
  useEffect(() => {
    if (!wanted) return undefined;
    let live = true;
    host
      .call<FlowDef[] | { defs?: FlowDef[] }>("flow.defs", project.trim().startsWith("/") ? { project: project.trim() } : {})
      .then((v) => live && setFlows(Array.isArray(v) ? v : (v?.defs ?? [])))
      .catch(() => live && setFlows([]));
    return () => {
      live = false;
    };
  }, [host, project, wanted]);
  return flows;
}

/** The registered projects' directories, for the project box's suggestions; none when the product has no such list. */
function useProjects(host: HostApi, wanted: boolean): { path: string; title?: string | null }[] {
  const [projects, setProjects] = useState<{ path: string; title?: string | null }[]>([]);
  useEffect(() => {
    if (!wanted) return undefined;
    let live = true;
    host
      .call<{ workspaces?: { path: string; title?: string | null }[] }>("harnix-app.workspaces.list", {})
      .then((v) => live && setProjects(v?.workspaces ?? []))
      .catch(() => live && setProjects([]));
    return () => {
      live = false;
    };
  }, [host, wanted]);
  return projects;
}

export function ScheduleEditor({ host, ctx, close }: { host: HostApi; ctx: EditorContext; close(result: Schedule | null): void }) {
  const tt = useT();
  const hello = useHello();
  const [base, setBase] = useState<Schedule | undefined>(ctx.existing);
  // When the draft was made: what it fills from the clock, and what a newer revision is compared at.
  const [opened] = useState(() => Date.now());
  const [draft, setDraft] = useState<Draft>(() => initialDraft(ctx, opened));
  const [touched, setTouched] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [conflict, setConflict] = useState(false);
  const [reviewed, setReviewed] = useState<{ revision: number; fields: string[] } | null>(null);
  const kinds: TargetKind[] = ctx.existing ? [ctx.existing.target.kind] : ctx.kinds;
  const fixedProject = ctx.project ?? null;
  const flows = useFlows(host, draft.project, draft.kind === "task");
  const projects = useProjects(host, fixedProject === null && draft.kind !== "session");
  const flow = flows.find((f) => f.name === draft.flow);
  const schema = JSON.stringify(flow?.inputs ?? null);
  const fields: InputField[] = useMemo(() => inputFields(flow?.inputs), [schema]);
  // A flow's fields known: their boxes start at what the schedule or the draft had, else the defaults.
  const [seeded, setSeeded] = useState<string | null>(null);
  useEffect(() => {
    if (!flow || seeded === `${flow.name}:${schema}`) return;
    const seed = seeded === null ? seedInputs(ctx) : undefined;
    setDraft((d) => ({ ...d, inputs: inputTexts(fields, seed) }));
    setSeeded(`${flow.name}:${schema}`);
  }, [flow?.name, schema]);

  const set = <K extends keyof Draft>(key: K, value: Draft[K]) => setDraft((d) => ({ ...d, [key]: value }));
  const setFreq = (patch: Partial<Frequency>) => setDraft((d) => ({ ...d, freq: { ...d.freq, ...patch } }));
  const problems = draftProblems(draft, { ...ctx, existing: base }, fields);
  const shown = (key: string) => (touched && problems[key] ? <div className="hx-sched-error" data-testid={`schedule-problem-${key}`}>{tt(problems[key]!)}</div> : null);
  const ok = Object.keys(problems).length === 0;

  const save = async () => {
    setTouched(true);
    if (!ok || busy) return;
    setBusy(true);
    setError(null);
    try {
      let saved: Schedule;
      if (base) {
        const params = updateParams(draft, ctx, base, fields);
        saved = changesAnything(params) ? await host.call<Schedule>("schedule.update", params) : base;
      } else {
        saved = await host.call<Schedule>("schedule.create", createParams(draft, ctx, fields));
      }
      rememberZone(draft.zone);
      close(saved);
    } catch (e) {
      if (base && errorCode(e) === CONFLICT) setConflict(true);
      else setError(errorText(e));
      setBusy(false);
    }
  };

  // Keep these changes over the current revision: what the other device changed is named, the draft stays.
  const review = async (keep: boolean) => {
    if (!base) return;
    setBusy(true);
    try {
      const current = await host.call<Schedule>("schedule.get", { id: base.id });
      if (keep) {
        setDraft((d) => rebase(d, ctx, base, current, opened));
        setReviewed({ revision: current.revision, fields: changedFields(base, current) });
      } else {
        setDraft(initialDraft({ ...ctx, existing: current }));
        setSeeded(null);
        setReviewed(null);
      }
      setBase(current);
      setConflict(false);
      setError(null);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const sessionLabel = ctx.session?.name || (ctx.session ? sessionName(host, ctx.session.instance, ctx.session.session_id) : base?.target.kind === "session" ? sessionName(host, base.target.instance, base.target.session_id) : tt("schedule.session.untitled"));
  const scenes = ((hello?.scenes ?? []) as Scene[]).filter((s) => s.active !== false);
  const f = draft.freq;
  const timeBox = (
    <label className="hx-sched-inline">
      <span>{tt("schedule.editor.time")}</span>
      <input className="hx-sched-input" type="time" data-testid="schedule-time" value={f.time} onChange={(e) => setFreq({ time: e.target.value })} />
    </label>
  );
  let summary = "";
  try {
    summary = Object.keys(problems).some((k) => ["when", "after", "every", "weekdays", "day", "time", "expr", "zone"].includes(k)) ? "" : describeTiming(toTiming(f), draft.zone);
  } catch {
    summary = "";
  }

  return (
    <>
      <DialogBody>
        <div className="hx-sched-editor" data-testid="schedule-editor" data-kind={draft.kind}>
          <div className="field">
            <label htmlFor="hx-sched-title">{tt("schedule.editor.title")}</label>
            <input id="hx-sched-title" className="hx-sched-input" data-testid="schedule-title" value={draft.title} maxLength={240} onChange={(e) => set("title", e.target.value)} />
            {draft.kind === "new_session" ? <div className="hint">{tt("schedule.editor.titleHint")}</div> : null}
            {shown("title")}
          </div>

          <div className="field">
            <label>{tt("schedule.editor.target")}</label>
            {kinds.length > 1 ? (
              <div className="seg hx-sched-seg" role="group">
                {kinds.map((k) => (
                  <button type="button" key={k} className={draft.kind === k ? "on" : ""} aria-pressed={draft.kind === k} data-testid={`schedule-target-${k}`} onClick={() => set("kind", k)}>
                    {tt(`schedule.editor.target.${k}`)}
                  </button>
                ))}
              </div>
            ) : (
              <div className="hx-sched-faint" data-testid="schedule-target">
                {tt(`schedule.editor.target.${draft.kind}`)}
              </div>
            )}
          </div>

          {draft.kind === "session" ? (
            <div className="hint" data-testid="schedule-session-target">
              {tt("schedule.editor.sessionIs", { name: sessionLabel })}
              {shown("session")}
            </div>
          ) : (
            <div className="field">
              <label htmlFor="hx-sched-project">{tt("schedule.editor.project")}</label>
              {fixedProject !== null && !ctx.existing ? (
                <div className="hx-sched-mono" data-testid="schedule-project-fixed" title={fixedProject}>
                  {fixedProject}
                </div>
              ) : (
                <>
                  <input
                    id="hx-sched-project"
                    className="hx-sched-input hx-sched-mono"
                    data-testid="schedule-project"
                    list="hx-sched-projects"
                    value={draft.project}
                    placeholder={draft.kind === "new_session" ? tt("schedule.editor.noProject") : "/"}
                    onChange={(e) => set("project", e.target.value)}
                  />
                  <datalist id="hx-sched-projects">
                    {projects.map((p) => (
                      <option key={p.path} value={p.path}>
                        {p.title ?? baseName(p.path)}
                      </option>
                    ))}
                  </datalist>
                  <div className="hint">{tt("schedule.editor.projectHint")}</div>
                </>
              )}
              {shown("project")}
            </div>
          )}

          {draft.kind === "new_session" ? (
            <>
              <div className="field">
                <label>{tt("schedule.editor.mode")}</label>
                <div className="seg hx-sched-seg" role="group">
                  {(["new_once_then_reuse", "new_each"] as const).map((m) => (
                    <button type="button" key={m} className={draft.mode === m ? "on" : ""} aria-pressed={draft.mode === m} data-testid={`schedule-mode-${m}`} onClick={() => set("mode", m)}>
                      {tt(`schedule.editor.mode.${m}`)}
                    </button>
                  ))}
                </div>
              </div>
              <div className="field hx-sched-pair">
                <label className="hx-sched-inline">
                  <span>{tt("schedule.editor.scene")}</span>
                  <select data-testid="schedule-scene" value={draft.scene} onChange={(e) => set("scene", e.target.value)}>
                    <option value="">{tt("schedule.editor.sceneDefault")}</option>
                    {scenes.map((s) => (
                      <option key={s.scene} value={s.scene}>
                        {s.name || s.scene}
                      </option>
                    ))}
                    {draft.scene && !scenes.some((s) => s.scene === draft.scene) ? <option value={draft.scene}>{draft.scene}</option> : null}
                  </select>
                </label>
                <label className="hx-sched-inline">
                  <span>{tt("schedule.editor.model")}</span>
                  <input className="hx-sched-input" data-testid="schedule-model" value={draft.model} placeholder={tt("schedule.editor.modelHint")} onChange={(e) => set("model", e.target.value)} />
                </label>
              </div>
            </>
          ) : null}

          {draft.kind !== "task" ? (
            <div className="field">
              <label htmlFor="hx-sched-prompt">{tt("schedule.editor.prompt")}</label>
              <textarea id="hx-sched-prompt" className="hx-sched-input" data-testid="schedule-prompt" rows={3} value={draft.prompt} onChange={(e) => set("prompt", e.target.value)} />
              {shown("prompt")}
            </div>
          ) : (
            <>
              <div className="field">
                <label htmlFor="hx-sched-flow">{tt("schedule.editor.flow")}</label>
                <select id="hx-sched-flow" data-testid="schedule-flow" value={draft.flow} onChange={(e) => set("flow", e.target.value)}>
                  <option value="">{tt("schedule.editor.flowPick")}</option>
                  {flows.map((d) => (
                    <option key={d.name} value={d.name} disabled={d.valid === false && d.name !== draft.flow}>
                      {d.valid === false ? tt("schedule.editor.flowInvalid", { title: d.title || d.name }) : d.title || d.name}
                    </option>
                  ))}
                  {draft.flow && !flow ? <option value={draft.flow}>{draft.flow}</option> : null}
                </select>
                {shown("flow")}
              </div>
              {fields.map((field) => (
                <div key={field.key} className="field" data-testid={`schedule-input-${field.key}`} data-kind={field.kind}>
                  <label htmlFor={`hx-sched-input-${field.key}`}>
                    {field.key}
                    {field.required ? " *" : ""}
                  </label>
                  {field.description ? <div className="hint">{field.description}</div> : null}
                  <InputBox field={field} id={`hx-sched-input-${field.key}`} value={draft.inputs[field.key] ?? ""} onChange={(v) => set("inputs", { ...draft.inputs, [field.key]: v })} />
                  {shown(`input.${field.key}`)}
                </div>
              ))}
              <div className="field">
                <label htmlFor="hx-sched-subject">{tt("schedule.editor.subject")} *</label>
                <textarea id="hx-sched-subject" className="hx-sched-input" data-testid="schedule-subject" rows={2} value={draft.subject} onChange={(e) => set("subject", e.target.value)} />
                {shown("subject")}
              </div>
              <div className="field hx-sched-pair">
                <label className="hx-sched-inline">
                  <span>{tt("schedule.editor.taskTitle")}</span>
                  <input className="hx-sched-input" data-testid="schedule-task-title" value={draft.taskTitle} onChange={(e) => set("taskTitle", e.target.value)} />
                </label>
                <label className="hx-sched-inline">
                  <span>{tt("schedule.editor.outputDir")}</span>
                  <input className="hx-sched-input hx-sched-mono" data-testid="schedule-output-dir" value={draft.outputDir} placeholder="flows/…" onChange={(e) => set("outputDir", e.target.value)} />
                </label>
              </div>
              <div className="hint">{tt("schedule.editor.outputDirHint")}</div>
              <div className="field">
                <label className="hx-sched-check">
                  <input type="checkbox" data-testid="schedule-worktree" checked={draft.workspace === "worktree"} onChange={(e) => set("workspace", e.target.checked ? "worktree" : "project")} />
                  {tt("schedule.editor.worktree")}
                </label>
                {draft.workspace === "" ? <div className="hint">{tt("schedule.editor.worktreeHint")}</div> : null}
              </div>
              <div className="field">
                <label>{tt("schedule.editor.concurrency")}</label>
                <div className="seg hx-sched-seg" role="group">
                  {(["skip", "parallel"] as const).map((c) => (
                    <button type="button" key={c} className={draft.concurrency === c ? "on" : ""} aria-pressed={draft.concurrency === c} data-testid={`schedule-concurrency-${c}`} onClick={() => set("concurrency", c)}>
                      {tt(`schedule.editor.concurrency.${c}`)}
                    </button>
                  ))}
                </div>
                {draft.concurrency === "parallel" ? <div className="hint">{tt("schedule.editor.parallelHint")}</div> : null}
              </div>
            </>
          )}

          <div className="field" data-testid="schedule-frequency" data-freq={f.kind}>
            <label>{tt("schedule.editor.frequency")}</label>
            <div className="seg hx-sched-seg" role="group">
              {FREQUENCY_KINDS.map((k) => (
                <button type="button" key={k} className={f.kind === k ? "on" : ""} aria-pressed={f.kind === k} data-testid={`schedule-freq-${k}`} onClick={() => setFreq({ kind: k })}>
                  {tt(`schedule.editor.freq.${k}`)}
                </button>
              ))}
            </div>
            <div className="hx-sched-pair">
              {f.kind === "once" ? (
                <>
                  <label className="hx-sched-inline">
                    <span>{tt("schedule.editor.date")}</span>
                    <input className="hx-sched-input" type="date" data-testid="schedule-date" value={f.date} onChange={(e) => setFreq({ date: e.target.value })} />
                  </label>
                  {timeBox}
                </>
              ) : null}
              {f.kind === "after" || f.kind === "every" ? (
                <>
                  <label className="hx-sched-inline">
                    <span>{tt("schedule.editor.amount")}</span>
                    <input
                      className="hx-sched-input hx-sched-number"
                      type="number"
                      min={1}
                      step={1}
                      data-testid={`schedule-${f.kind}`}
                      value={Number.isFinite(f.kind === "after" ? f.after : f.every) ? (f.kind === "after" ? f.after : f.every) : ""}
                      onChange={(e) => setFreq(f.kind === "after" ? { after: Number(e.target.value) } : { every: Number(e.target.value) })}
                    />
                  </label>
                  <label className="hx-sched-inline">
                    <span>{tt("schedule.editor.unit")}</span>
                    <select
                      data-testid={`schedule-${f.kind}-unit`}
                      value={f.kind === "after" ? f.afterUnit : f.everyUnit}
                      onChange={(e) => setFreq(f.kind === "after" ? { afterUnit: e.target.value as Frequency["afterUnit"] } : { everyUnit: e.target.value as Frequency["everyUnit"] })}
                    >
                      {(f.kind === "after" ? (["minutes", "hours", "days"] as const) : (["minutes", "hours"] as const)).map((u) => (
                        <option key={u} value={u}>
                          {tt(`schedule.editor.unit.${u}`)}
                        </option>
                      ))}
                    </select>
                  </label>
                </>
              ) : null}
              {f.kind === "daily" || f.kind === "workdays" ? timeBox : null}
              {f.kind === "weekly" ? (
                <>
                  <div className="hx-sched-weekdays" role="group" aria-label={tt("schedule.editor.weekdays")}>
                    {[1, 2, 3, 4, 5, 6, 7].map((d) => {
                      const on = f.weekdays.includes(d);
                      return (
                        <button
                          type="button"
                          key={d}
                          className={`hx-sched-weekday${on ? " on" : ""}`}
                          aria-pressed={on}
                          data-testid={`schedule-weekday-${d}`}
                          onClick={() => setFreq({ weekdays: on ? f.weekdays.filter((x) => x !== d) : [...f.weekdays, d].sort((a, b) => a - b) })}
                        >
                          {tt(`schedule.wd.${d}`)}
                        </button>
                      );
                    })}
                  </div>
                  {timeBox}
                </>
              ) : null}
              {f.kind === "monthly" ? (
                <>
                  <label className="hx-sched-inline">
                    <span>{tt("schedule.editor.day")}</span>
                    <input className="hx-sched-input hx-sched-number" type="number" min={1} max={31} step={1} data-testid="schedule-day" value={Number.isFinite(f.day) ? f.day : ""} onChange={(e) => setFreq({ day: Number(e.target.value) })} />
                  </label>
                  {timeBox}
                </>
              ) : null}
              {f.kind === "cron" ? (
                <label className="hx-sched-inline hx-sched-grow">
                  <span>{tt("schedule.editor.cron")}</span>
                  <input className="hx-sched-input hx-sched-mono" data-testid="schedule-cron" value={f.expr} onChange={(e) => setFreq({ expr: e.target.value })} />
                </label>
              ) : null}
            </div>
            {f.kind === "monthly" && f.day > 28 ? <div className="hint">{tt("schedule.editor.dayHint")}</div> : null}
            {f.kind === "cron" ? <div className="hint">{tt("schedule.editor.cronHint")}</div> : null}
            {["when", "after", "every", "weekdays", "day", "time", "expr"].map((k) => (problems[k] ? <div key={k}>{shown(k)}</div> : null))}
            {summary ? (
              <div className="hx-sched-summary" data-testid="schedule-freq-summary">
                {summary}
              </div>
            ) : null}
          </div>

          <div className="field">
            <label>{tt("schedule.editor.zone")}</label>
            <ZonePicker value={draft.zone} onChange={(zone) => set("zone", zone)} invalid={Boolean(problems.zone)} />
            {shown("zone")}
          </div>

          <div className="field">
            <label>{tt("schedule.editor.missed")}</label>
            <div className="seg hx-sched-seg" role="group">
              {(["catch_up_latest", "skip"] as const).map((m) => (
                <button type="button" key={m} className={draft.missed === m ? "on" : ""} aria-pressed={draft.missed === m} data-testid={`schedule-missed-${m}`} onClick={() => set("missed", m)}>
                  {tt(`schedule.editor.missed.${m}`)}
                </button>
              ))}
            </div>
            <div className="hint">{tt("schedule.notRunning")}</div>
          </div>

          {conflict ? (
            <div className="hx-sched-note is-warn" data-testid="schedule-conflict" role="alert">
              <span>{tt("schedule.editor.conflict")}</span>
              <button type="button" className="btn sm primary" data-testid="schedule-conflict-keep" disabled={busy} onClick={() => void review(true)}>
                {tt("schedule.editor.conflictKeep")}
              </button>
              <button type="button" className="btn sm ghost" data-testid="schedule-conflict-discard" disabled={busy} onClick={() => void review(false)}>
                {tt("schedule.editor.conflictDiscard")}
              </button>
            </div>
          ) : null}
          {reviewed ? (
            <div className="hx-sched-note" data-testid="schedule-reviewed" data-revision={reviewed.revision} role="status">
              {reviewed.fields.length
                ? tt("schedule.editor.reviewed", { revision: reviewed.revision, fields: reviewed.fields.map((x) => tt(`schedule.field.${x}`)).join(tt("schedule.wd.sep")) })
                : tt("schedule.editor.reviewedNothing", { revision: reviewed.revision })}
            </div>
          ) : null}
          {error ? (
            <div className="hx-sched-error" data-testid="schedule-error" role="alert">
              {error}
            </div>
          ) : null}
        </div>
      </DialogBody>
      <DialogFoot>
        <button type="button" className="btn outline" onClick={() => close(null)}>
          {tt("common.cancel")}
        </button>
        <button type="button" className="btn primary" data-testid="schedule-save" disabled={busy || conflict || (touched && !ok)} onClick={() => void save()}>
          {tt(base ? "schedule.editor.save" : "schedule.editor.create")}
        </button>
      </DialogFoot>
    </>
  );
}

/** One input's box, by its kind. */
function InputBox({ field, id, value, onChange }: { field: InputField; id: string; value: string; onChange(v: string): void }) {
  switch (field.kind) {
    case "boolean":
      return <input id={id} type="checkbox" checked={value === "true"} onChange={(e) => onChange(e.target.checked ? "true" : "false")} />;
    case "enum":
      return (
        <select id={id} value={value} onChange={(e) => onChange(e.target.value)}>
          {!field.options?.includes(value) ? <option value="">—</option> : null}
          {(field.options ?? []).map((o) => (
            <option key={o} value={o}>
              {o}
            </option>
          ))}
        </select>
      );
    case "number":
      return <input id={id} className="hx-sched-input" type="number" step="any" value={value} onChange={(e) => onChange(e.target.value)} />;
    case "json":
      return <textarea id={id} className="hx-sched-input hx-sched-mono" rows={2} value={value} placeholder="{}" onChange={(e) => onChange(e.target.value)} />;
    default:
      return <input id={id} className="hx-sched-input" value={value} onChange={(e) => onChange(e.target.value)} />;
  }
}
