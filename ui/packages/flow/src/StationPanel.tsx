/**
 * One station's details (component B, flow-design §17), in two tabs.
 *
 * Definition: what the station is — its work, skill (with the body), scene, the output contract,
 * exit conditions, rework targets, writable tools, hooks, and the protocol layer when the module
 * gives it. Run: what happened to it in this run — status and rounds, what it still lacks, its
 * tasks (claimant, output, error, a link to the session), condition results, confirmations, the
 * forced record, its history — and the hands to act: confirm, reject, force, rework, release, retry,
 * and for an external task, claim and hand in its output. A button that does not apply to the
 * station's status is disabled. A dry run's session task also shows the message its session would
 * have been sent (`preview`, xops-02 §5).
 */

import { useEffect, useState, type ReactNode } from "react";
import { useNest, useT } from "@harnix/react";

import { errorText } from "./hooks";
import {
  actorLabel,
  checkOk,
  digestLines,
  formatDuration,
  formatTime,
  formatTokens,
  leaseUntil,
  methodName,
  pretty,
  stationActions,
  stationElapsed,
  stationStyle,
  stationUsage,
  stationTitle,
  sessionOf,
  taskTone,
  tokensOf,
  workLabel,
} from "./model";
import type { DefInfo, Executor, FlowDef, FlowMethod, History, RunView, Station, TaskRec, TaskSpec } from "./types";

export interface StationPanelProps {
  station: string;
  /** The definition: the run's pinned one when a run is shown. */
  def: FlowDef;
  view?: RunView | null;
  /** From `def {name}`: skill bodies and, when given, the protocol layer. */
  info?: DefInfo | null;
  /** The module id the application registered the flow module under. */
  module?: string;
  initialTab?: "definition" | "run";
}

type Form = { kind: "reject" | "force" | "rework" } | null;

export function StationPanel({ station, def, view, info, module = "flow", initialTab }: StationPanelProps) {
  const t = useT();
  const [tab, setTab] = useState<"definition" | "run">(initialTab ?? (view ? "run" : "definition"));
  const s = def.stations.find((x) => x.id === station);
  if (!s) {
    return (
      <div className="hx-flow-panel" data-testid="flow-station-panel">
        <p className="hx-flow-hint">{t("flowui.panel.noStation", { station })}</p>
      </div>
    );
  }
  const rec = view?.record.stations[station];
  const style = stationStyle(view ? (rec?.status ?? "pending") : undefined);
  return (
    <div className="hx-flow-panel" data-testid="flow-station-panel" data-station={station}>
      <div className="hx-flow-panel-head">
        <div className="hx-flow-panel-title">{stationTitle(s)}</div>
        <code className="hx-flow-id">{s.id}</code>
        {view ? (
          <span className={`hx-flow-pill ${style.className}`} data-testid="flow-station-status" data-status={rec?.status ?? "pending"}>
            {t("flowui.status." + (rec?.status ?? "pending"))}
          </span>
        ) : null}
      </div>
      <div className="seg hx-flow-seg" role="tablist">
        <button type="button" role="tab" aria-selected={tab === "definition"} className={tab === "definition" ? "on" : ""} data-testid="flow-tab-definition" onClick={() => setTab("definition")}>
          {t("flowui.panel.definition")}
        </button>
        <button type="button" role="tab" aria-selected={tab === "run"} className={tab === "run" ? "on" : ""} data-testid="flow-tab-run" disabled={!view} onClick={() => setTab("run")}>
          {t("flowui.panel.run")}
        </button>
      </div>
      {tab === "run" && view ? <RunTab station={s} view={view} module={module} /> : <DefinitionTab station={s} def={def} info={info ?? null} />}
    </div>
  );
}

/* ── definition ──────────────────────────────────────────────────────── */

function Field({ label, children, testid }: { label: string; children: ReactNode; testid?: string }) {
  return (
    <div className="hx-flow-field" data-testid={testid}>
      <div className="hx-flow-label">{label}</div>
      <div className="hx-flow-value">{children}</div>
    </div>
  );
}

function Code({ value, testid }: { value: unknown; testid?: string }) {
  return (
    <pre className="hx-flow-code" data-testid={testid}>
      {pretty(value)}
    </pre>
  );
}

function executorText(e: Executor): string {
  switch (e.kind) {
    case "session": {
      if (!e.scene && (e.session ?? "main") === "main") {
        return "session · main session";
      }
      const alloc = e.session ?? "new";
      const where = [
        e.instance ? `instance ${e.instance}` : "",
        e.template ? `template ${e.template}` : "",
        e.space ? `space ${e.space}` : "",
        e.model ? `model ${e.provider ? `${e.provider}/` : ""}${e.model}` : e.provider ? `provider ${e.provider}` : "",
      ].filter(Boolean);
      return [`session · scene ${e.scene}`, ...where, typeof alloc === "string" ? alloc : `key ${alloc.key}`].join(" · ");
    }
    case "agent":
      return ["agent · sub-agent of the main session", e.agent_type ? `type ${e.agent_type}` : "", e.model ? `model ${e.model}` : ""].filter(Boolean).join(" · ");
    case "program":
      return `program · step ${e.step}`;
    case "external":
      return `external${e.who ? ` · ${e.who}` : ""}`;
    case "subflow":
      return `subflow · ${e.flow}`;
    default:
      return pretty(e);
  }
}

function DefinitionTab({ station: s, def, info }: { station: Station; def: FlowDef; info: DefInfo | null }) {
  const t = useT();
  const index = def.stations.findIndex((x) => x.id === s.id);
  const needs = Array.isArray(s.needs) ? s.needs : index > 0 ? [def.stations[index - 1]!.id] : [];
  const specs: TaskSpec[] = s.tasks?.length ? s.tasks : s.executor ? [{ id: s.id, executor: s.executor, skill: s.skill ?? null }] : [];
  const skills = [...new Set(specs.map((x) => x.skill).filter((x): x is string => !!x))];
  const protocol = info?.protocol?.[s.id];
  return (
    <div className="hx-flow-tab" data-testid="flow-definition">
      {s.description ? <p className="hx-flow-desc">{s.description}</p> : null}
      <Field label={t("flowui.def.needs")}>{needs.length ? needs.join(", ") : t("flowui.def.root")}</Field>
      {s.when?.length ? (
        <Field label={t("flowui.def.when")}>
          <Code value={s.when} />
        </Field>
      ) : null}
      <Field label={t("flowui.def.work")}>
        {specs.length === 0 ? (
          t("flowui.graph.gate")
        ) : (
          <ul className="hx-flow-list">
            {s.foreach ? (
              <li>
                {t("flowui.def.foreach")} <code>{s.foreach}</code>
                {s.max_parallel ? ` · ${t("flowui.def.maxParallel", { n: s.max_parallel })}` : ""}
              </li>
            ) : null}
            {specs.map((x) => (
              <li key={x.id}>
                {s.tasks?.length ? <code>{x.id}</code> : null} {executorText(x.executor)}
                {x.skill ? ` · skill ${x.skill}` : ""}
              </li>
            ))}
            <li className="hx-flow-faint">{workLabel(s)}</li>
          </ul>
        )}
      </Field>
      {s.retry ? <Field label={t("flowui.def.retry")}>{t("flowui.def.retryMax", { n: s.retry.max })}</Field> : null}
      {skills.map((name) => (
        <Field key={name} label={t("flowui.def.skill", { name })} testid={`flow-skill-${name}`}>
          {info?.skills?.[name] !== undefined ? <pre className="hx-flow-code hx-flow-prose">{info.skills[name]}</pre> : <span className="hx-flow-faint">{t("flowui.def.noBody")}</span>}
        </Field>
      ))}
      {protocol ? (
        <Field label={t("flowui.def.protocol")} testid="flow-protocol">
          <pre className="hx-flow-code hx-flow-prose">{protocol}</pre>
        </Field>
      ) : null}
      <Field label={t("flowui.def.output")}>{s.output ? <Code value={s.output} testid="flow-output-contract" /> : <span className="hx-flow-faint">{t("flowui.def.none")}</span>}</Field>
      <Field label={t("flowui.def.exit")}>
        <ul className="hx-flow-list" data-testid="flow-exit">
          {(s.exit ?? []).map((c, i) => (
            <li key={i}>
              <strong>{t("flowui.exit." + c.kind)}</strong>
              {c.kind === "tool" ? (
                <>
                  {" "}
                  <code>{c.tool}</code>
                  {c.args || c.assert ? <Code value={{ args: c.args ?? undefined, assert: c.assert ?? undefined }} /> : null}
                </>
              ) : c.kind === "gate" ? (
                <>
                  {" "}
                  <code>{c.gate}</code>
                </>
              ) : c.kind === "confirm" && c.who ? (
                ` · ${c.who}`
              ) : null}
            </li>
          ))}
        </ul>
      </Field>
      {s.rework_to?.length || s.on_unmet ? (
        <Field label={t("flowui.def.rework")}>
          {s.rework_to?.length ? <div>{t("flowui.def.reworkTo", { list: s.rework_to.join(", ") })}</div> : null}
          {s.on_unmet ? <div>{t("flowui.def.onUnmet", { to: s.on_unmet.rework_to, max: s.on_unmet.max ?? "—" })}</div> : null}
        </Field>
      ) : null}
      {s.hold ? <Field label={t("flowui.def.hold")}>{t("flowui.def.holdYes")}</Field> : null}
      <Field label={t("flowui.def.writes")}>{s.writes?.length ? s.writes.map((w) => <code key={w} className="hx-flow-chip">{w}</code>) : <span className="hx-flow-faint">{t("flowui.def.readOnly")}</span>}</Field>
      {s.hooks?.length ? (
        <Field label={t("flowui.def.hooks")}>
          <ul className="hx-flow-list">
            {s.hooks.map((h, i) => (
              <li key={i}>
                <code>{h.on}</code> → <code>{h.action}</code>
              </li>
            ))}
          </ul>
        </Field>
      ) : null}
    </div>
  );
}

/* ── run ─────────────────────────────────────────────────────────────── */

function RunTab({ station: s, view, module }: { station: Station; view: RunView; module: string }) {
  const t = useT();
  const nest = useNest();
  const run = view.record;
  const rec = run.stations[s.id];
  const actions = stationActions(run.status, s, rec);
  const unmet = view.unmet?.[s.id] ?? [];
  const [form, setForm] = useState<Form>(null);
  const [reason, setReason] = useState("");
  const [target, setTarget] = useState<string>("");
  const [tasks, setTasks] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [history, setHistory] = useState<History[] | null>(null);
  // External tasks this panel claimed and has not handed in: their tokens, by task id.
  const [held, setHeld] = useState<Record<string, Claim>>({});
  const hold = (task: string, claim: Claim | null) =>
    setHeld((h) => {
      const next = { ...h };
      if (claim) next[task] = claim;
      else delete next[task];
      return next;
    });

  // The station's history, read again whenever the run changes.
  useEffect(() => {
    let live = true;
    nest
      .call<History[] | { history?: History[] }>(methodName(module, "history"), { run: run.id })
      .then((h) => live && setHistory(Array.isArray(h) ? h : (h?.history ?? [])))
      .catch(() => live && setHistory(null));
    return () => {
      live = false;
    };
  }, [nest, module, run.id, run.rev]);

  const act = async (name: FlowMethod, params: Record<string, unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await nest.call(methodName(module, name), { run: run.id, ...params });
      setForm(null);
      setReason("");
      setTasks([]);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const open = (kind: "reject" | "force" | "rework") => {
    setError(null);
    setReason("");
    setTasks([]);
    if (kind === "rework") setTarget(actions.rework[0] ?? "");
    setForm(form?.kind === kind ? null : { kind });
  };

  const submit = () => {
    if (!form) return;
    if (form.kind === "reject") void act("confirm", { station: s.id, pass: false, reason: reason.trim() || undefined });
    if (form.kind === "force") void act("force", { station: s.id, reason: reason.trim() });
    if (form.kind === "rework") void act("rework", { to: target, tasks: tasks.length ? tasks : undefined, reason: reason.trim() });
  };
  const needsReason = form?.kind === "force" || form?.kind === "rework";
  const targetTasks = form?.kind === "rework" && target ? (run.stations[target]?.tasks ?? []) : [];

  return (
    <div className="hx-flow-tab" data-testid="flow-run">
      <div className="hx-flow-actions" data-testid="flow-actions">
        <button type="button" className="btn sm primary" data-testid="flow-confirm" disabled={busy || !actions.confirm} onClick={() => void act("confirm", { station: s.id, pass: true })}>
          {t("flowui.act.confirm")}
        </button>
        <button type="button" className="btn sm outline" data-testid="flow-reject" disabled={busy || !actions.confirm} onClick={() => open("reject")}>
          {t("flowui.act.reject")}
        </button>
        <button type="button" className="btn sm outline" data-testid="flow-force" disabled={busy || !actions.force} onClick={() => open("force")}>
          {t("flowui.act.force")}
        </button>
        <button type="button" className="btn sm outline" data-testid="flow-rework" disabled={busy || !actions.rework.length} onClick={() => open("rework")}>
          {t("flowui.act.rework")}
        </button>
        <button type="button" className="btn sm outline" data-testid="flow-release" disabled={busy || !actions.release} onClick={() => void act("release", { station: s.id })}>
          {t("flowui.act.release")}
        </button>
      </div>
      {form ? (
        <div className="hx-flow-form" data-testid="flow-action-form" data-kind={form.kind}>
          {form.kind === "rework" ? (
            <>
              <label className="hx-flow-label" htmlFor="hx-flow-rework-target">
                {t("flowui.act.reworkTo")}
              </label>
              <select id="hx-flow-rework-target" data-testid="flow-rework-target" value={target} onChange={(e) => {
                  setTarget(e.target.value);
                  setTasks([]);
                }}>
                {actions.rework.map((id) => (
                  <option key={id} value={id}>
                    {id}
                  </option>
                ))}
              </select>
              {targetTasks.length > 1 ? (
                <fieldset className="hx-flow-tasks-pick" data-testid="flow-rework-tasks">
                  <legend className="hx-flow-label">{t("flowui.act.reworkTasks")}</legend>
                  {targetTasks.map((task) => (
                    <label key={task.id} className="hx-flow-check">
                      <input
                        type="checkbox"
                        data-testid={`flow-rework-task-${task.id}`}
                        checked={tasks.includes(task.id)}
                        onChange={(e) => setTasks(e.target.checked ? [...tasks, task.id] : tasks.filter((x) => x !== task.id))}
                      />
                      {task.id}
                    </label>
                  ))}
                </fieldset>
              ) : null}
            </>
          ) : null}
          <textarea
            className="hx-flow-input"
            data-testid="flow-reason"
            rows={2}
            value={reason}
            placeholder={t(needsReason ? "flowui.act.reasonRequired" : "flowui.act.reasonOptional")}
            onChange={(e) => setReason(e.target.value)}
          />
          <div className="hx-flow-actions">
            <button type="button" className="btn sm primary" data-testid="flow-action-submit" disabled={busy || (needsReason && !reason.trim()) || (form.kind === "rework" && !target)} onClick={submit}>
              {t("flowui.act.submit." + form.kind)}
            </button>
            <button type="button" className="btn sm ghost" onClick={() => setForm(null)}>
              {t("flowui.act.cancel")}
            </button>
          </div>
        </div>
      ) : null}
      {error ? (
        <div className="hx-flow-error" data-testid="flow-error" role="alert">
          {error}
        </div>
      ) : null}

      {!rec ? (
        <p className="hx-flow-hint">{t("flowui.run.notEntered")}</p>
      ) : (
        <>
          <Field label={t("flowui.run.round")}>
            {t("flowui.run.roundOf", { round: rec.round, total: rec.rounds_total ?? rec.round })}
            {rec.auto_reworks ? ` · ${t("flowui.run.autoReworks", { n: rec.auto_reworks })}` : ""}
          </Field>
          {rec.entered_at || rec.left_at ? (
            <Field label={t("flowui.run.times")}>
              {formatTime(rec.entered_at)}
              {rec.left_at ? ` → ${formatTime(rec.left_at)}` : ""}
              <span className="hx-flow-faint" data-testid="flow-station-spent" data-tokens={tokensOf(stationUsage(rec))}>
                {[` · ${formatDuration(stationElapsed(rec, Date.now()))}`, tokensOf(stationUsage(rec)) ? t("flowui.tokens", { n: formatTokens(tokensOf(stationUsage(rec))) }) : ""].filter(Boolean).join(" · ")}
              </span>
            </Field>
          ) : null}
          {digestLines(rec.skill_digest).length ? (
            <Field label={t("flowui.run.skillDigest")}>
              {digestLines(rec.skill_digest).map((line) => (
                <div key={line}>
                  <code className="hx-flow-id">{line}</code>
                </div>
              ))}
            </Field>
          ) : null}
          {unmet.length && rec.status !== "passed" && rec.status !== "skipped" ? (
            <Field label={t("flowui.run.unmet")} testid="flow-unmet">
              <ul className="hx-flow-list">
                {unmet.map((u, i) => (
                  <li key={i}>
                    <strong>{u.label || t("flowui.exit." + u.kind)}</strong> {u.detail !== undefined && u.detail !== null && u.detail !== "" ? pretty(u.detail) : ""}
                  </li>
                ))}
              </ul>
            </Field>
          ) : null}
          {rec.forced ? (
            <Field label={t("flowui.run.forced")} testid="flow-forced">
              {t("flowui.run.forcedBy", { by: actorLabel(rec.forced.by), at: formatTime(rec.forced.at) })}
              <div>{rec.forced.reason}</div>
              {rec.forced.unmet?.length ? <div className="hx-flow-faint">{rec.forced.unmet.map((u) => u.kind).join(", ")}</div> : null}
            </Field>
          ) : null}
          <Field label={t("flowui.run.tasks", { n: (rec.tasks ?? []).length })}>
            {!rec.tasks?.length ? (
              <span className="hx-flow-faint">{t("flowui.run.noTasks")}</span>
            ) : (
              <ul className="hx-flow-tasks">
                {rec.tasks.map((task) => {
                  const session = sessionOf(task);
                  return (
                    <li key={task.id} className="hx-flow-task" data-testid={`flow-task-${task.id}`} data-status={task.status}>
                      <div className="hx-flow-task-head">
                        <code>{task.id}</code>
                        <span className={`hx-flow-pill hx-flow-tone-${taskTone(task.status)}`}>{t("flowui.task." + task.status)}</span>
                        {task.attempt > 1 ? <span className="hx-flow-faint">{t("flowui.run.attempt", { n: task.attempt })}</span> : null}
                        <TaskSpent task={task} />
                        <span className="hx-flow-grow" />
                        {session ? (
                          <button type="button" className="btn sm ghost" data-testid={`flow-open-session-${task.id}`} onClick={() => void nest.session.open(session, { switchSpace: true })}>
                            {t("flowui.run.openSession")}
                          </button>
                        ) : null}
                        {actions.retry.includes(task.id) ? (
                          <button type="button" className="btn sm outline" data-testid={`flow-retry-${task.id}`} disabled={busy} onClick={() => void act("retry", { station: s.id, task: task.id })}>
                            {t("flowui.act.retry")}
                          </button>
                        ) : null}
                      </div>
                      {task.item !== undefined && task.item !== null ? <div className="hx-flow-faint">{t("flowui.run.item")}: {clipped(task.item)}</div> : null}
                      {task.lease ? (
                        <div className="hx-flow-faint" data-testid={`flow-lease-${task.id}`}>
                          {leaseUntil(task.lease.until) === null
                            ? t("flowui.run.leaseChild", { by: actorLabel(task.lease.by) })
                            : t("flowui.run.lease", { by: actorLabel(task.lease.by), until: formatTime(task.lease.until) })}
                        </div>
                      ) : null}
                      {task.child ? <div className="hx-flow-faint">{t("flowui.run.child", { run: task.child })}</div> : null}
                      {task.error ? <div className="hx-flow-error">{task.error}</div> : null}
                      {task.preview ? (
                        <details className="hx-flow-details" data-testid={`flow-preview-${task.id}`}>
                          <summary>
                            {t("flowui.run.preview")}
                            <span className="hx-flow-faint">
                              {" "}
                              {[task.preview.scene, task.preview.delivery].filter(Boolean).join(" · ")}
                            </span>
                          </summary>
                          <pre className="hx-flow-code hx-flow-prose" data-testid={`flow-preview-message-${task.id}`}>
                            {task.preview.message}
                          </pre>
                        </details>
                      ) : null}
                      {executorOf(s, task.id)?.kind === "external" && (task.status === "ready" || (task.status === "claimed" && held[task.id])) ? (
                        <HandIn module={module} run={run.id} station={s.id} task={task.id} held={held[task.id] ?? null} onHeld={(c) => hold(task.id, c)} />
                      ) : null}
                      {task.output !== undefined && task.output !== null ? (
                        <details className="hx-flow-details">
                          <summary>{t("flowui.run.output")}</summary>
                          <Code value={task.output} />
                        </details>
                      ) : null}
                    </li>
                  );
                })}
              </ul>
            )}
          </Field>
          {rec.checks?.length ? (
            <Field label={t("flowui.run.checks")} testid="flow-checks">
              <ul className="hx-flow-list">
                {rec.checks.map((c, i) => (
                  <li key={i} className={`hx-flow-tone-${checkOk(c) ? "ok" : c.status === "wait" ? "muted" : "error"}`} data-status={c.status ?? (c.ok ? "pass" : "fail")}>
                    {checkOk(c) ? "✓" : c.status === "wait" ? "…" : "✗"} {c.label || t("flowui.exit." + c.kind)} {c.detail !== undefined && c.detail !== null && c.detail !== "" ? <span className="hx-flow-faint">{clipped(c.detail)}</span> : null}
                  </li>
                ))}
              </ul>
            </Field>
          ) : null}
          {rec.confirms?.length ? (
            <Field label={t("flowui.run.confirms")} testid="flow-confirms">
              <ul className="hx-flow-list">
                {rec.confirms.map((c, i) => (
                  <li key={i}>
                    {t(c.pass ? "flowui.run.confirmed" : "flowui.run.rejected", { by: actorLabel(c.by), at: formatTime(c.at) })}
                    {c.reason ? ` — ${c.reason}` : ""}
                  </li>
                ))}
              </ul>
            </Field>
          ) : null}
        </>
      )}
      {history ? (
        <Field label={t("flowui.run.history")} testid="flow-station-history">
          <ul className="hx-flow-list hx-flow-history">
            {history
              .filter((h) => h.station === s.id)
              .map((h) => (
                <li key={h.seq}>
                  <span className="hx-flow-faint">{formatTime(h.at)}</span> <strong>{h.op}</strong>
                  {h.task ? ` · ${h.task}` : ""} · {actorLabel(h.by)}
                  {h.detail !== undefined && h.detail !== null ? <span className="hx-flow-faint"> {clipped(h.detail)}</span> : null}
                </li>
              ))}
          </ul>
        </Field>
      ) : null}
    </div>
  );
}

/** How long a task's attempt took (still takes) and its tokens. */
function TaskSpent({ task }: { task: TaskRec }) {
  const t = useT();
  const from = typeof task.started_at === "number" ? task.started_at : null;
  const took = from === null ? null : (typeof task.ended_at === "number" ? task.ended_at : Date.now()) - from;
  const tokens = tokensOf(task.usage);
  if (took === null && !tokens) return null;
  return (
    <span className="hx-flow-faint" data-testid={`flow-task-spent-${task.id}`} data-tokens={tokens}>
      {[formatDuration(took), tokens ? t("flowui.tokens", { n: formatTokens(tokens) }) : ""].filter(Boolean).join(" · ")}
    </span>
  );
}

/** The executor a station's task runs under: its own in `tasks`, else the station's. */
function executorOf(s: Station, task: string): Executor | null {
  return s.tasks?.find((x) => x.id === task)?.executor ?? s.executor ?? null;
}

/** A claim this panel holds: the token `submit` needs, and when it lapses. */
interface Claim {
  token: string;
  until: number;
}

/**
 * An external task done by the person here: claim it, hand in its output. The claim is kept while
 * the hand-in is refused — the error is shown, and it can be handed in again with the same token
 * until the claim lapses (it then goes back to ready by itself), or given up now (recorded as a
 * failed attempt, which can be retried).
 */
function HandIn({ module, run, station, task, held, onHeld }: { module: string; run: string; station: string; task: string; held: Claim | null; onHeld(claim: Claim | null): void }) {
  const t = useT();
  const nest = useNest();
  const [open, setOpen] = useState(held !== null);
  const [output, setOutput] = useState("{}");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const live = held && (leaseUntil(held.until) ?? Infinity) > Date.now() ? held : null;
  const submit = async () => {
    let parsed: unknown;
    try {
      parsed = JSON.parse(output);
    } catch (e) {
      setError(t("flowui.start.badJson", { error: errorText(e) }));
      return;
    }
    setBusy(true);
    setError(null);
    let claim = live;
    try {
      if (!claim) {
        claim = await nest.call<Claim>(methodName(module, "claim"), { run, station, task });
        onHeld(claim);
      }
      await nest.call(methodName(module, "submit"), { run, station, task, token: claim.token, output: parsed });
      onHeld(null);
      setOpen(false);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const giveUp = async () => {
    if (!live) return onHeld(null);
    setBusy(true);
    setError(null);
    try {
      await nest.call(methodName(module, "fail"), { run, station, task, token: live.token, error: t("flowui.act.handInGivenUp") });
      onHeld(null);
      setOpen(false);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  if (!open) {
    return (
      <div className="hx-flow-actions">
        <button type="button" className="btn sm outline" data-testid={`flow-hand-in-${task}`} onClick={() => setOpen(true)}>
          {t("flowui.act.handIn")}
        </button>
      </div>
    );
  }
  return (
    <div className="hx-flow-form" data-testid={`flow-hand-in-form-${task}`} data-held={live ? "true" : undefined}>
      <label className="hx-flow-label" htmlFor={`hx-flow-hand-in-${task}`}>
        {t("flowui.act.handInOutput")}
      </label>
      <textarea id={`hx-flow-hand-in-${task}`} className="hx-flow-input hx-flow-mono" data-testid={`flow-hand-in-output-${task}`} rows={3} value={output} onChange={(e) => setOutput(e.target.value)} />
      {live ? (
        <div className="hx-flow-faint" data-testid={`flow-hand-in-held-${task}`}>
          {t("flowui.act.handInHeld", { until: formatTime(live.until) })}
        </div>
      ) : null}
      {error ? (
        <div className="hx-flow-error" role="alert" data-testid={`flow-hand-in-error-${task}`}>
          {error}
        </div>
      ) : null}
      <div className="hx-flow-actions">
        <button type="button" className="btn sm primary" data-testid={`flow-hand-in-submit-${task}`} disabled={busy} onClick={() => void submit()}>
          {t(live ? "flowui.act.submit.handInAgain" : "flowui.act.submit.handIn")}
        </button>
        {live ? (
          <button type="button" className="btn sm outline" data-testid={`flow-hand-in-give-up-${task}`} disabled={busy} onClick={() => void giveUp()}>
            {t("flowui.act.handInGiveUp")}
          </button>
        ) : (
          <button type="button" className="btn sm ghost" onClick={() => setOpen(false)}>
            {t("flowui.act.cancel")}
          </button>
        )}
      </div>
    </div>
  );
}

function clipped(v: unknown): string {
  const text = typeof v === "string" ? v : JSON.stringify(v);
  return text && text.length > 160 ? `${text.slice(0, 159)}…` : (text ?? "");
}
