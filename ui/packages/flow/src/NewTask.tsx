/**
 * The new-task dialog, from a project's "+" menu (or `/flow` in a conversation, on its flow and
 * subject, `slash.tsx`): a flow (every one on offer, its origin marked,
 * one that does not validate disabled with why), a form drawn from the flow's `inputs` schema, the
 * subject, a title, the output directory (the default shown, as the module will resolve it), and
 * whether to run in a worktree (the flow's `workspace` to begin with; only for a project that is a
 * git repository, asked of the module by `workspace_info`, the reason shown when it is not); then
 * `start {flow, subject, inputs, title, project_root, …}` and the task opened.
 *
 * The form is deliberately small: a string, a number, a boolean, an enumeration, a list of strings;
 * anything else is a JSON field. The schema is the module's (`DefRow.inputs`,
 * `FlowDef::inputs_schema`): one object, a property per input, `default` on the optional ones.
 */

import { useEffect, useMemo, useState } from "react";
import { DialogBody, DialogFoot, Glyph, t, useHost, useT, type HostApi, type Registry } from "@harnix/react";

import { errorText, useDefs, useMethod } from "./hooks";
import { defBlockedKey, methodName, originLabel, runIdOf, runningIn, sameRoot } from "./model";
import { StartActions, type StartDraft } from "./points";
import type { DefInfo, DefSummary, RunSummary, StartedFrom, WorkspaceInfo } from "./types";

/**
 * What the dialog sends as `workspace`: nothing while the toggle was not touched (the flow's own
 * `workspace`, else the project), else what it says. A project that is not a git repository sends
 * nothing: a flow that asks for a worktree there is refused by the module, with why.
 */
export function workspaceParam(git: boolean, picked: boolean | null): "worktree" | "project" | undefined {
  if (!git || picked === null) return undefined;
  return picked ? "worktree" : "project";
}

/** Why a project cannot have a worktree, in a sentence: the reason's own, else git's words. */
export function noWorktreeText(info: WorkspaceInfo | undefined): string | null {
  if (!info || info.git) return null;
  const known = ["not_a_repository", "bare", "no_commit", "no_git"];
  return known.includes(info.reason) ? t("flowui.task.worktreeNo." + info.reason) : (info.detail ?? info.reason);
}

export type FieldKind = "string" | "number" | "integer" | "boolean" | "enum" | "strings" | "json";

/** One input of a flow, as the form draws it. */
export interface SchemaField {
  key: string;
  kind: FieldKind;
  required: boolean;
  description?: string;
  options?: string[];
  default?: unknown;
}

/** The fields of an `inputs` schema, in the schema's order. */
export function fieldsOf(schema: unknown): SchemaField[] {
  if (!schema || typeof schema !== "object") return [];
  const s = schema as { properties?: Record<string, unknown>; required?: unknown };
  const required = new Set(Array.isArray(s.required) ? s.required.filter((x): x is string => typeof x === "string") : []);
  return Object.entries(s.properties ?? {}).map(([key, raw]) => {
    const p = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
    const field: SchemaField = { key, kind: "json", required: required.has(key) };
    if (typeof p.description === "string") field.description = p.description;
    if (p.default !== undefined) field.default = p.default;
    const type = Array.isArray(p.type) ? p.type.find((x) => x !== "null") : p.type;
    if (Array.isArray(p.enum) && p.enum.every((x) => typeof x === "string")) {
      field.kind = "enum";
      field.options = p.enum as string[];
    } else if (type === "string") field.kind = "string";
    else if (type === "number") field.kind = "number";
    else if (type === "integer") field.kind = "integer";
    else if (type === "boolean") field.kind = "boolean";
    else if (type === "array" && (p.items as { type?: unknown } | undefined)?.type === "string") field.kind = "strings";
    return field;
  });
}

/** What a field's box starts with: its default, as text. */
export function initialText(field: SchemaField): string {
  const d = field.default;
  if (d === undefined || d === null) return "";
  switch (field.kind) {
    case "string":
    case "enum":
      return typeof d === "string" ? d : JSON.stringify(d);
    case "number":
    case "integer":
      return typeof d === "number" ? String(d) : "";
    case "boolean":
      return d === true ? "true" : "";
    case "strings":
      return Array.isArray(d) ? d.map(String).join("\n") : "";
    case "json":
      return JSON.stringify(d, null, 2);
  }
}

/**
 * The inputs from what was typed: a field's value, or why it cannot be read. An optional field
 * left blank is not sent (the module applies its default); a required one left blank is an error.
 */
export function collectInputs(fields: SchemaField[], texts: Record<string, string>): { inputs: Record<string, unknown>; errors: Record<string, string> } {
  const inputs: Record<string, unknown> = {};
  const errors: Record<string, string> = {};
  for (const f of fields) {
    const text = (texts[f.key] ?? "").trim();
    if (f.kind === "boolean") {
      if (text === "true") inputs[f.key] = true;
      else if (f.required || text === "false") inputs[f.key] = false;
      continue;
    }
    if (!text) {
      if (f.required) errors[f.key] = t("flowui.task.required");
      continue;
    }
    switch (f.kind) {
      case "string":
      case "enum":
        inputs[f.key] = text;
        break;
      case "number":
      case "integer": {
        const n = Number(text);
        if (!Number.isFinite(n) || (f.kind === "integer" && !Number.isInteger(n))) errors[f.key] = t("flowui.task.notNumber");
        else inputs[f.key] = n;
        break;
      }
      case "strings":
        inputs[f.key] = text
          .split("\n")
          .map((x) => x.trim())
          .filter(Boolean);
        break;
      case "json":
        try {
          inputs[f.key] = JSON.parse(text);
        } catch (e) {
          errors[f.key] = t("flowui.task.notJson", { error: errorText(e) });
        }
        break;
    }
  }
  return { inputs, errors };
}

/** `YYYYMMDD` of a moment, local time. */
function dateStamp(now: number): string {
  const d = new Date(now);
  return `${d.getFullYear()}${String(d.getMonth() + 1).padStart(2, "0")}${String(d.getDate()).padStart(2, "0")}`;
}

/**
 * The output directory the module will give a task by default — `flows/<flow>-<YYYYMMDD><NN>`, NN
 * the day's next number — as this page can foresee it: one more than the runs of that flow in that
 * project started today. The module decides; this is what the dialog shows as the placeholder.
 */
export function defaultOutputDir(flow: string, runs: RunSummary[], root: string, now = Date.now()): string {
  const day = dateStamp(now);
  const today = runs.filter((r) => r.flow === flow && sameRoot(r.project_root, root) && dateStamp(Number(r.created_at ?? 0)) === day).length;
  return `flows/${flow}-${day}${String(today + 1).padStart(2, "0")}`;
}

/** A flow's top-level `guard`, `output_dir` and `workspace`, read without assuming they are there. */
export function defOptions(info: DefInfo | undefined): { guard: string | null; outputDir: string | null; workspace: string | null } {
  const def = (info?.def ?? {}) as Record<string, unknown>;
  const str = (v: unknown) => (typeof v === "string" && v ? v : null);
  return { guard: str(def.guard), outputDir: str(def.output_dir), workspace: str(def.workspace) };
}

/* ── the dialog ───────────────────────────────────────────────────────── */

export interface NewTaskOptions {
  /** The project's directory. */
  root: string;
  /** The backend space the task's main session is shown in besides the flow's (`shown_in`):
   *  the one the projects space lists, so the task opens there. Absent: shown nowhere else. */
  space?: string;
  /** The runs as last known: the warning about one still going, and the output directory's number. */
  runs(): RunSummary[];
  /** The flow to start on, when there is one. */
  flow?: string;
  /** The subject to start with (what `/flow <flow> <subject>` said). */
  subject?: string;
  /** The conversation it is started from (`/flow`): the module writes a card linking to the task there. */
  from?: StartedFrom;
  /** Where the other ways to start are (`flow.start.action`), offered beside starting now with what
   *  was filled in; none, none offered. */
  registry?: Registry;
}

/** Open the dialog; `onStarted` is called with the run once it is. */
export async function newTaskDialog(host: HostApi, module: string, options: NewTaskOptions, onStarted: (run: string, flow: string) => void | Promise<void>): Promise<void> {
  await host.dialogs.open<true>((close) => <NewTaskForm host={host} module={module} options={options} onStarted={onStarted} close={close} />, { title: t("flowui.task.new"), wide: true });
}

function NewTaskForm({ host, module, options, onStarted, close }: { host: HostApi; module: string; options: NewTaskOptions; onStarted: (run: string, flow: string) => void | Promise<void>; close: (result: true | null) => void }) {
  const t = useT();
  const nest = useHost();
  const defs = useDefs(module, options.root);
  const [picked, setPicked] = useState<string | null>(options.flow ?? null);
  const flow = picked ?? defs.list.find((d) => d.valid !== false)?.name ?? null;
  const def = defs.list.find((d) => d.name === flow);
  const info = useMethod<DefInfo>(module, "def", flow && def?.valid !== false ? { name: flow, project: options.root } : null, def?.digest);
  const { guard, outputDir: definedDir, workspace: definedWorkspace } = defOptions(info.value);
  const repo = useMethod<WorkspaceInfo>(module, "workspace_info", { project_root: options.root });
  const git = repo.value?.git === true;
  const noWorktree = noWorktreeText(repo.value) ?? (repo.error ? repo.error : null);
  // By the schema's text, not its object: the list pushed again with the same schema keeps what was typed.
  const schema = JSON.stringify(def?.inputs ?? null);
  const fields = useMemo(() => fieldsOf(def?.inputs), [schema]);
  const [texts, setTexts] = useState<Record<string, string>>({});
  // A flow picked: its fields start at their defaults.
  useEffect(() => setTexts(Object.fromEntries(fields.map((f) => [f.key, initialText(f)]))), [flow, schema]);
  const [subject, setSubject] = useState(options.subject ?? "");
  const [title, setTitle] = useState("");
  const [outputDir, setOutputDir] = useState("");
  // Untouched (null): as the flow says.
  const [worktree, setWorktree] = useState<boolean | null>(null);
  useEffect(() => setWorktree(null), [flow]);
  const inWorktree = git && (worktree ?? definedWorkspace === "worktree");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [touched, setTouched] = useState(false);

  const runs = options.runs();
  const running = runningIn(runs, options.root);
  const placeholder = flow ? (definedDir ?? defaultOutputDir(flow, runs, options.root)) : "";
  const collected = collectInputs(fields, texts);
  const problems = Object.keys(collected.errors).length;
  const canStart = Boolean(flow && def?.valid !== false && subject.trim() && !problems && !busy);

  const start = async () => {
    setTouched(true);
    if (!canStart || !flow) return;
    setBusy(true);
    setError(null);
    try {
      const params: Record<string, unknown> = {
        flow,
        subject: subject.trim(),
        project_root: options.root,
        inputs: Object.keys(collected.inputs).length ? collected.inputs : undefined,
        title: title.trim() || undefined,
        output_dir: outputDir.trim() || undefined,
        // Sent only when the toggle was touched in a git repository; the flow's own otherwise.
        workspace: workspaceParam(git, worktree),
        shown_in: options.space || undefined,
        from_session: options.from,
      };
      const run = runIdOf(await nest.call(methodName(module, "start"), params));
      if (!run) throw new Error(t("flowui.task.noRun"));
      close(true);
      await onStarted(run, flow);
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  };

  // What another way to start takes: what is filled in so far, a blank left out; the subject may be blank yet.
  const draft = (): StartDraft | null =>
    flow
      ? {
          flow,
          subject: subject.trim() || undefined,
          inputs: Object.keys(collected.inputs).length ? collected.inputs : undefined,
          title: title.trim() || undefined,
          output_dir: outputDir.trim() || undefined,
          workspace: workspaceParam(git, worktree),
          project_root: options.root,
          shown_in: options.space || undefined,
        }
      : null;

  const set = (key: string, value: string) => setTexts((s) => ({ ...s, [key]: value }));
  return (
    <>
      <DialogBody>
        <div className="field" data-testid="task-flows">
          <label>{t("flowui.start.flow")}</label>
          {defs.error ? <div className="hx-flow-error">{defs.error}</div> : null}
          {defs.loaded && !defs.list.length ? <div className="hint">{t("flowui.list.noFlows")}</div> : null}
          <div className="menu-list hx-flow-task-flows">
            {defs.list.map((d) => (
              <FlowChoice key={d.name} def={d} on={d.name === flow} pick={() => setPicked(d.name)} />
            ))}
          </div>
          {def?.description ? <div className="hint">{def.description}</div> : null}
        </div>
        {fields.map((f) => (
          <div key={f.key} className="field" data-testid={`task-input-${f.key}`} data-kind={f.kind}>
            <label htmlFor={`hx-task-input-${f.key}`}>
              {f.key}
              {f.required ? <span className="hx-flow-task-required"> *</span> : null}
            </label>
            {f.description ? <div className="hint">{f.description}</div> : null}
            <Field field={f} id={`hx-task-input-${f.key}`} value={texts[f.key] ?? ""} onChange={(v) => set(f.key, v)} />
            {touched && collected.errors[f.key] ? <div className="hx-flow-error">{collected.errors[f.key]}</div> : null}
          </div>
        ))}
        <div className="field">
          <label htmlFor="hx-task-subject">
            {t("flowui.start.subject")}
            <span className="hx-flow-task-required"> *</span>
          </label>
          <textarea id="hx-task-subject" className="hx-flow-input" data-testid="task-subject" rows={3} value={subject} placeholder={t("flowui.task.subjectHint")} onChange={(e) => setSubject(e.target.value)} />
        </div>
        <div className="field">
          <label htmlFor="hx-task-title">{t("flowui.start.title")}</label>
          <input id="hx-task-title" className="hx-flow-input" data-testid="task-title" value={title} onChange={(e) => setTitle(e.target.value)} />
        </div>
        <div className="field">
          <label htmlFor="hx-task-output-dir">{t("flowui.task.outputDir")}</label>
          <input id="hx-task-output-dir" className="hx-flow-input hx-flow-mono" data-testid="task-output-dir" value={outputDir} placeholder={placeholder} onChange={(e) => setOutputDir(e.target.value)} />
          <div className="hint">{t("flowui.task.outputDirHint", { dir: placeholder })}</div>
        </div>
        <div className="field">
          <label className="hx-flow-task-check">
            <input type="checkbox" data-testid="task-worktree" checked={inWorktree} disabled={!git} onChange={(e) => setWorktree(e.target.checked)} />
            {t("flowui.task.worktree")}
          </label>
          {git ? <div className="hint">{t("flowui.task.worktreeHint")}</div> : null}
          {!git && noWorktree ? (
            <div className="hint" data-testid="task-worktree-reason">
              {noWorktree}
            </div>
          ) : null}
          {!git && repo.value && definedWorkspace === "worktree" ? (
            <div className="hx-flow-task-note hx-flow-tone-warn" data-testid="task-worktree-needed" role="status">
              <Glyph name="alert" className="glyph" />
              {t("flowui.task.worktreeNeeded")}
            </div>
          ) : null}
        </div>
        {running.length ? (
          <div className="hx-flow-task-note hx-flow-tone-warn" data-testid="task-running-warning" role="status">
            <Glyph name="alert" className="glyph" />
            {t("flowui.task.alreadyRunning", { title: running[0]!.title || running[0]!.subject || running[0]!.id, count: running.length })}
          </div>
        ) : null}
        {guard !== "strict" ? (
          <div className="hx-flow-task-note" data-testid="task-modifies" data-worktree={inWorktree ? "true" : undefined}>
            <Glyph name="info" className="glyph" />
            {t(inWorktree ? "flowui.task.modifiesWorktree" : "flowui.task.modifies")}
          </div>
        ) : null}
        {error ? (
          <div className="hx-flow-error" data-testid="task-error" role="alert">
            {error}
          </div>
        ) : null}
      </DialogBody>
      <DialogFoot>
        {options.registry ? <StartActions registry={options.registry} draft={draft} disabled={!flow || def?.valid === false || problems > 0 || busy} before={() => close(null)} testid="task-action" /> : null}
        <button type="button" className="btn outline" onClick={() => close(null)}>
          {t("common.cancel")}
        </button>
        <button type="button" className="btn primary" data-testid="task-start" disabled={!canStart} onClick={() => void start()}>
          {t("flowui.start.submit")}
        </button>
      </DialogFoot>
    </>
  );
}

/** A flow on offer: title, name, origin; one that does not validate cannot be picked. */
function FlowChoice({ def, on, pick }: { def: DefSummary; on: boolean; pick(): void }) {
  const t = useT();
  const invalid = def.valid === false;
  // Why it cannot be started: an untrusted project's own flow waits on trust ("trust the project first").
  const why = def.reason ? t(defBlockedKey(def)!) : t("flowui.task.invalid");
  return (
    <button type="button" className={on ? "on" : ""} data-testid={`task-flow-${def.name}`} data-origin={def.origin} data-valid={invalid ? "false" : "true"} aria-pressed={on} disabled={invalid} data-reason={def.reason ?? undefined} title={invalid ? why : def.name} onClick={pick}>
      <Glyph name="fork" className="glyph" />
      <span className="truncate">{def.title || def.name}</span>
      <span className="hx-flow-badge">{originLabel(t, def)}</span>
      {invalid ? <span className="sub">{why}</span> : <span className="sub">{def.name}</span>}
    </button>
  );
}

/** One input's box, by its kind. */
function Field({ field, id, value, onChange }: { field: SchemaField; id: string; value: string; onChange(v: string): void }) {
  const t = useT();
  switch (field.kind) {
    case "boolean":
      return (
        <label className="hx-flow-task-check">
          <input id={id} type="checkbox" checked={value === "true"} onChange={(e) => onChange(e.target.checked ? "true" : "false")} />
          {t("flowui.task.yes")}
        </label>
      );
    case "enum":
      return (
        <select id={id} value={value} onChange={(e) => onChange(e.target.value)}>
          {field.required && !field.options?.includes(value) ? <option value="">—</option> : null}
          {(field.options ?? []).map((o) => (
            <option key={o} value={o}>
              {o}
            </option>
          ))}
        </select>
      );
    case "number":
    case "integer":
      return <input id={id} className="hx-flow-input" type="number" step={field.kind === "integer" ? 1 : "any"} value={value} onChange={(e) => onChange(e.target.value)} />;
    case "strings":
      return <textarea id={id} className="hx-flow-input" rows={3} value={value} placeholder={t("flowui.task.onePerLine")} onChange={(e) => onChange(e.target.value)} />;
    case "json":
      return <textarea id={id} className="hx-flow-input hx-flow-mono" rows={3} value={value} placeholder="{}" onChange={(e) => onChange(e.target.value)} />;
    default:
      return <input id={id} className="hx-flow-input" value={value} onChange={(e) => onChange(e.target.value)} />;
  }
}
