/**
 * A run's details, shared by the two places that show one: the panel beside the conversation in
 * the flow space and the system console (`RunDetail` in `tabs.tsx`), and the right column's 流程
 * tab of a task's main session (`TaskTab.tsx`).
 *
 *   PhaseBadge       a run's state as a pill
 *   RunHead          title, id, state; pause / resume, cancel with a reason
 *   DryControls      a dry run's hands: the next piece of work, or everything up to a person
 *   RunStations      every station on a row: status, time, tokens, its output in a line, and the
 *                    results of its gates; a row clicked picks the station
 *   OutputDirLine    where the deliverables go (a task's `output_dir`): copy the path, open the files tab
 *   RunSessionsList  the run's main session, sub-agents and tasks, each opened in the middle
 *   RunPanel         all of them with the graph and the picked station's panel, following the
 *                    station at work until a person picks one
 *
 * Every part draws from the `run {run}` topic (`useRunView`) and acts through the module's methods.
 */

import { useEffect, useState } from "react";
import { useHost, useT, type HostSessionRef as SessionRef } from "@harnix/react";

import { FlowGraph } from "./FlowGraph";
import { errorText, useMethod, useRunView } from "./hooks";
import {
  checkOk,
  defOf,
  formatDuration,
  formatTime,
  formatTokens,
  isDryRun,
  methodName,
  phaseOf,
  phaseTone,
  pretty,
  runElapsed,
  runLive,
  runUsage,
  sessionOf,
  stationElapsed,
  stationStyle,
  stationTitle,
  stationUsage,
  tokensOf,
} from "./model";
import { StationPanel } from "./StationPanel";
import type { CheckRec, DryProgress, Phase, RunSessions, RunStatus, RunView, SessionWire, StationRec } from "./types";

/** Now, again every second while `live`: elapsed times of what is still going. */
export function useNow(live: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!live) return undefined;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [live]);
  return now;
}

export function asRef(s: SessionWire | null | undefined): SessionRef | null {
  return s && typeof s.instance === "string" && typeof s.session_id === "string" ? { instance: s.instance, sessionId: s.session_id } : null;
}

/**
 * The station a run is at: the first of the definition's order that is being worked on, held or
 * unmet; none while nothing is (not started, or over).
 */
export function currentStation(view: RunView | null | undefined): string | null {
  const def = defOf(view);
  if (!def) return null;
  const open = (rec: StationRec | undefined) => rec?.status === "working" || rec?.status === "unmet" || rec?.status === "held";
  return def.stations.find((s) => open(view!.record.stations[s.id]))?.id ?? null;
}

/**
 * Where a run is, as "station k of n": the current station's place in the definition's order,
 * the last once it is over, and 0 before it has entered any.
 */
export function taskStep(view: RunView | null | undefined): { k: number; n: number } | null {
  const def = defOf(view);
  if (!def) return null;
  const n = def.stations.length;
  const current = currentStation(view);
  if (current) return { k: def.stations.findIndex((s) => s.id === current) + 1, n };
  if (!runLive({ phase: view!.phase, status: view!.record.status })) return { k: n, n };
  const entered = def.stations.filter((s) => view!.record.stations[s.id]?.entered_at != null).length;
  return { k: entered, n };
}

/** A run's deliverables directory, as its record holds it; null when it has none. */
export function outputDirOf(view: RunView | null | undefined): string | null {
  const dir = (view?.record as { output_dir?: unknown } | undefined)?.output_dir;
  return typeof dir === "string" && dir ? dir : null;
}

/** The reasons a run waits for that have a sentence of their own (`flowui.waiting.*`). */
const WAITING = new Set(["approval", "confirm", "held", "unmet", "failed", "external", "children", "work"]);

/** A run's state as a pill: a spinner while it is pausing; why it waits, when it does, in one sentence. */
export function PhaseBadge({ phase, testid }: { phase: Phase; testid?: string }) {
  const t = useT();
  return (
    <span className={`hx-flow-pill hx-flow-phase hx-flow-tone-${phaseTone(phase.state)}`} data-testid={testid} data-phase={phase.state} data-reason={phase.reason ?? undefined}>
      {phase.state === "pausing" || phase.state === "running" ? <span className={`hx-flow-spin${phase.state === "running" ? " hx-flow-spin-slow" : ""}`} aria-hidden="true" /> : null}
      {phase.state === "waiting" && phase.reason && WAITING.has(phase.reason) ? t("flowui.waiting." + phase.reason) : t("flowui.phase." + phase.state)}
    </span>
  );
}

/* ── the head ─────────────────────────────────────────────────────────── */

export function RunHead({ view, module }: { view: RunView; module: string }) {
  const t = useT();
  const nest = useHost();
  const run = view.record;
  const active = run.status === "active";
  const now = useNow(active);
  const [cancelling, setCancelling] = useState(false);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const call = async (name: "cancel" | "pause" | "resume", params: Record<string, unknown> = {}) => {
    setBusy(true);
    setError(null);
    try {
      await nest.call(methodName(module, name), { run: run.id, ...params });
      if (name === "cancel") {
        setCancelling(false);
        setReason("");
      }
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const shown: Phase = phaseOf({ phase: view.phase, status: run.status });
  const tokens = tokensOf(runUsage(run));
  return (
    <>
      <div className="hx-flow-head">
        <div className="hx-flow-head-title">{run.title || run.subject || run.id}</div>
        <code className="hx-flow-id">{run.id}</code>
        <PhaseBadge phase={shown} testid="flow-run-phase" />
        <span className="hx-flow-grow" />
        {active && !isDryRun(run.id) ? (
          run.paused ? (
            <button type="button" className="btn sm outline" data-testid="flow-resume-run" disabled={busy} onClick={() => void call("resume")}>
              {t("flowui.act.resume")}
            </button>
          ) : (
            <button type="button" className="btn sm outline" data-testid="flow-pause-run" disabled={busy} onClick={() => void call("pause")}>
              {t("flowui.act.pause")}
            </button>
          )
        ) : null}
        <button type="button" className="btn sm outline" data-testid="flow-cancel-run" disabled={!active} onClick={() => setCancelling(!cancelling)}>
          {t("flowui.act.cancelRun")}
        </button>
      </div>
      <div className="hx-flow-meta hx-flow-faint" data-testid="flow-run-meta" data-tokens={tokens}>
        {t("flowui.overview.meta", { flow: run.flow, subject: run.subject, at: formatTime(run.created_at), by: `${run.created_by?.kind ?? ""}:${run.created_by?.id ?? ""}` })}
        {` · ${t("flowui.overview.elapsed", { d: formatDuration(runElapsed(run, now)) })}`}
        {tokens ? ` · ${t("flowui.tokens", { n: formatTokens(tokens) })}` : ""}
        {run.parent ? ` · ${t("flowui.overview.parent", { run: run.parent.run, station: run.parent.station })}` : ""}
      </div>
      {cancelling ? (
        <div className="hx-flow-form" data-testid="flow-cancel-form">
          <textarea className="hx-flow-input" data-testid="flow-cancel-reason" rows={2} value={reason} placeholder={t("flowui.act.reasonRequired")} onChange={(e) => setReason(e.target.value)} />
          <div className="hx-flow-actions">
            <button type="button" className="btn sm primary" data-testid="flow-cancel-submit" disabled={!reason.trim() || busy} onClick={() => void call("cancel", { reason: reason.trim() })}>
              {t("flowui.act.submit.cancel")}
            </button>
            <button type="button" className="btn sm ghost" onClick={() => setCancelling(false)}>
              {t("flowui.act.cancel")}
            </button>
          </div>
        </div>
      ) : null}
      {error ? (
        <div className="hx-flow-error" role="alert">
          {error}
        </div>
      ) : null}
    </>
  );
}

/** A dry run's hands: take the next piece of work, or everything up to a person. */
export function DryControls({ module, run, status }: { module: string; run: string; status: RunStatus }) {
  const t = useT();
  const nest = useHost();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [last, setLast] = useState<DryProgress | null>(null);
  const go = async (how: "dry_run.step" | "dry_run.run") => {
    setBusy(true);
    setError(null);
    try {
      setLast(await nest.call<DryProgress>(methodName(module, how), { run }));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="hx-flow-form" data-testid="flow-dry-controls">
      <div className="hx-flow-actions">
        <span className="hx-flow-faint">{t("flowui.dry.note")}</span>
        <span className="hx-flow-grow" />
        <button type="button" className="btn sm outline" data-testid="flow-dry-step" disabled={busy || status !== "active"} onClick={() => void go("dry_run.step")}>
          {t("flowui.dry.step")}
        </button>
        <button type="button" className="btn sm primary" data-testid="flow-dry-run" disabled={busy || status !== "active"} onClick={() => void go("dry_run.run")}>
          {t("flowui.dry.run")}
        </button>
      </div>
      {error ? (
        <div className="hx-flow-error" role="alert">
          {error}
        </div>
      ) : null}
      {last ? (
        <div data-testid="flow-dry-last" data-pending={last.pending} data-steps={last.steps.length}>
          <div className="hx-flow-faint">{t("flowui.dry.took", { n: last.steps.length, pending: last.pending })}</div>
          {last.steps.length ? (
            <ul className="hx-flow-list hx-flow-history">
              {last.steps.map((s, i) => (
                <li key={i}>
                  <strong>{s.kind}</strong> {[s.station, s.task, s.gate].filter(Boolean).join(" · ")}
                  {s.result !== undefined && s.result !== null ? <span className="hx-flow-faint"> {typeof s.result === "string" ? s.result : JSON.stringify(s.result)}</span> : null}
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/* ── every station on a row ───────────────────────────────────────────── */

/** A task's output in one line, for the table; "" when it has none. */
function outputLine(rec: StationRec | undefined): string {
  const task = (rec?.tasks ?? []).find((x) => x.output !== undefined && x.output !== null);
  if (!task) return "";
  const text = typeof task.output === "string" ? task.output : pretty(task.output);
  const line = text.replace(/\s+/g, " ").trim();
  return line.length > 80 ? `${line.slice(0, 79)}…` : line;
}

function GateResults({ checks }: { checks: CheckRec[] }) {
  const t = useT();
  if (!checks.length) return null;
  return (
    <span className="hx-flow-task-gates">
      {checks.map((c, i) => (
        <span key={i} className={`hx-flow-chip hx-flow-tone-${checkOk(c) ? "ok" : c.status === "wait" ? "warn" : "error"}`} data-testid="flow-gate-result" data-status={c.status} title={c.detail ?? undefined}>
          {c.label || t("flowui.exit." + c.kind)}
          {` · ${t("flowui.task.check." + c.status)}`}
        </span>
      ))}
    </span>
  );
}

/** Every station of the run on a row: status, time, tokens, output, gate results. */
export function RunStations({ view, selected, onPick }: { view: RunView; selected: string | null; onPick(station: string): void }) {
  const t = useT();
  const def = defOf(view);
  const now = useNow(runLive({ phase: view.phase, status: view.record.status }));
  if (!def) return null;
  return (
    <table className="hx-flow-table hx-flow-task-stations" data-testid="flow-run-stations">
      <thead>
        <tr>
          <th>{t("flowui.task.station")}</th>
          <th>{t("flowui.task.status")}</th>
          <th>{t("flowui.task.spent")}</th>
          <th>{t("flowui.task.tokens")}</th>
          <th>{t("flowui.task.output")}</th>
        </tr>
      </thead>
      <tbody>
        {def.stations.map((s) => {
          const rec = view.record.stations[s.id];
          const style = stationStyle(rec?.status ?? "pending");
          const tokens = tokensOf(stationUsage(rec));
          return (
            <tr key={s.id} className={selected === s.id ? "on" : ""} data-testid={`flow-run-station-${s.id}`} data-status={rec?.status ?? "pending"} onClick={() => onPick(s.id)}>
              <td>
                <button type="button" className="hx-flow-link" aria-pressed={selected === s.id}>
                  {stationTitle(s)}
                </button>
              </td>
              <td>
                <span className={`hx-flow-pill ${style.className}`}>{t("flowui.status." + (rec?.status ?? "pending"))}</span>
              </td>
              <td>{formatDuration(stationElapsed(rec, now))}</td>
              <td>{tokens ? formatTokens(tokens) : ""}</td>
              <td className="hx-flow-task-output">
                {outputLine(rec)}
                <GateResults checks={rec?.checks ?? []} />
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

/* ── where the deliverables go ────────────────────────────────────────── */

/** The run's output directory, to copy, and to open in the files tab where there is one. */
export function OutputDirLine({ view, onOpenFiles }: { view: RunView; onOpenFiles?: (() => boolean) | null }) {
  const t = useT();
  const [copied, setCopied] = useState(false);
  const dir = outputDirOf(view);
  if (!dir) return null;
  const copy = () => {
    const clip = typeof navigator !== "undefined" ? navigator.clipboard : undefined;
    if (!clip) return;
    void clip.writeText(dir).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };
  return (
    <div className="hx-flow-task-output-dir" data-testid="flow-output-dir" data-path={dir}>
      <span className="hx-flow-faint">{t("flowui.task.outputDir")}</span>
      <code className="hx-flow-id" title={dir}>
        {dir}
      </code>
      <button type="button" className="btn sm ghost" data-testid="flow-output-dir-copy" onClick={copy}>
        {copied ? t("flowui.task.copied") : t("flowui.task.copyPath")}
      </button>
      {onOpenFiles ? (
        <button type="button" className="btn sm ghost" data-testid="flow-output-dir-files" onClick={() => onOpenFiles()}>
          {t("flowui.task.openFiles")}
        </button>
      ) : null}
    </div>
  );
}

/* ── sub-agents and tasks ─────────────────────────────────────────────── */

/** The run's sub-agents and tasks (`sessions {run}`): each opens in the middle. */
export function RunSessionsList({ module, run, rev }: { module: string; run: string; rev: number }) {
  const t = useT();
  const nest = useHost();
  const sessions = useMethod<RunSessions>(module, "sessions", { run }, rev);
  const v = sessions.value;
  const main = asRef(v?.main);
  const open = (ref: SessionRef | null) => {
    if (ref) void nest.session.open(ref);
  };
  const agents = v?.agents ?? [];
  const sidechains = (v?.sidechains ?? []).filter((c) => !agents.some((a) => a.agent_session_id === c.session_id));
  const tasks = (v?.tasks ?? []).filter((task) => !(task.kind === "agent" && task.flow && agents.some((a) => a.flow?.station === task.flow?.station && a.flow?.task === task.flow?.task)));
  const own = v?.own ?? [];
  return (
    <section className="hx-flow-section" data-testid="flow-run-sessions">
      <div className="hx-flow-section-head">{t("flowui.sessions.title")}</div>
      {sessions.error ? <div className="hx-flow-error">{sessions.error}</div> : null}
      <ul className="hx-flow-rows">
        {main ? (
          <li>
            <button type="button" className="hx-flow-row" data-testid="flow-open-main" onClick={() => open(main)}>
              <span className="hx-flow-row-title">{t("flowui.sessions.main")}</span>
              <span className="hx-flow-faint">{main.sessionId}</span>
            </button>
          </li>
        ) : v ? (
          <li className="hx-flow-hint">{t("flowui.sessions.noMain")}</li>
        ) : null}
        {agents.map((a) => (
          <li key={a.agent_session_id}>
            <button
              type="button"
              className="hx-flow-row"
              data-testid={`flow-agent-${a.agent_session_id}`}
              data-status={a.status ?? undefined}
              data-station={a.flow?.station}
              onClick={() => open({ instance: a.instance, sessionId: a.agent_session_id })}
            >
              <span className="hx-flow-row-title">
                <span className="hx-flow-badge">{t("flowui.sessions.kind.agent")}</span>
                {a.flow ? `${a.flow.station}/${a.flow.task}` : a.description || a.agent_session_id}
              </span>
              <span className="hx-flow-faint">{[a.agent_type ?? "", a.status ? t("flowui.sessions.status." + (a.status === "failed" ? "failed" : "done")) : t("flowui.sessions.status.running")].filter(Boolean).join(" · ")}</span>
            </button>
          </li>
        ))}
        {tasks.map((task) => {
          // What Agix lists of the main session's own tasks (a shell command, a monitor, a
          // sub-agent still running): they are the main session's, opened there.
          return (
            <li key={task.task_id}>
              <button
                type="button"
                className="hx-flow-row"
                data-testid={`flow-task-of-main-${task.task_id}`}
                data-kind={task.kind}
                data-status={task.status}
                disabled={!main}
                onClick={() => open(main)}
              >
                <span className="hx-flow-row-title">
                  <span className="hx-flow-badge">{t("flowui.sessions.kind." + (["agent", "teammate", "shell", "monitor", "cron", "code"].includes(task.kind) ? task.kind : "other"))}</span>
                  {task.flow ? `${task.flow.station}/${task.flow.task}` : task.detail?.description || task.detail?.command || task.task_id}
                </span>
                <span className="hx-flow-faint">
                  {[task.detail?.agent_type ?? "", task.status, formatDuration(task.started_ms ? (task.ended_ms ?? Date.now()) - task.started_ms : null)].filter(Boolean).join(" · ")}
                </span>
              </button>
            </li>
          );
        })}
        {sidechains.map((c) => (
          <li key={c.session_id}>
            <button type="button" className="hx-flow-row" data-testid={`flow-sidechain-${c.session_id}`} onClick={() => open({ instance: c.instance, sessionId: c.session_id })}>
              <span className="hx-flow-row-title">
                <span className="hx-flow-badge">{t("flowui.sessions.kind.agent")}</span>
                {c.name || c.title || c.session_id}
              </span>
              <span className="hx-flow-faint">{formatTime(c.created_at)}</span>
            </button>
          </li>
        ))}
        {own.map((o) => {
          const ref = asRef(o.session) ?? sessionOf({ session: o.session } as never);
          return (
            <li key={`${o.station}/${o.task}`}>
              <button type="button" className="hx-flow-row" data-testid={`flow-own-session-${o.station}-${o.task}`} onClick={() => open(ref)}>
                <span className="hx-flow-row-title">
                  <span className="hx-flow-badge">{t("flowui.sessions.own")}</span>
                  {`${o.station}/${o.task}`}
                </span>
                <span className="hx-flow-faint">{ref?.sessionId ?? ""}</span>
              </button>
            </li>
          );
        })}
      </ul>
      {v && !main && !agents.length && !tasks.length && !sidechains.length && !own.length ? <p className="hx-flow-hint">{t("flowui.sessions.none")}</p> : null}
    </section>
  );
}

/* ── all of it ────────────────────────────────────────────────────────── */

export interface RunPanelProps {
  module: string;
  run: string;
  /** Given: the output directory line offers to open the files tab; answers whether it could. */
  onOpenFiles?: (() => boolean) | null;
}

/**
 * The run in full: head, graph with the current station highlighted, the picked station's panel
 * (confirm, release, rework… where the station allows), every station on a row, the output
 * directory, the sub-agents and tasks. The station at work is picked until a person picks one.
 */
export function RunPanel({ module, run, onOpenFiles }: RunPanelProps) {
  const t = useT();
  const view = useRunView(module, run);
  const [picked, setPicked] = useState<{ station: string | null; byHand: boolean }>({ station: null, byHand: false });
  const current = currentStation(view);
  useEffect(() => {
    if (!picked.byHand && current && current !== picked.station) setPicked({ station: current, byHand: false });
  }, [current]);
  if (!view) return <p className="hx-flow-hint">{t("flowui.loading")}</p>;
  const def = defOf(view);
  const station = picked.station && def?.stations.some((s) => s.id === picked.station) ? picked.station : null;
  const pick = (s: string) => setPicked({ station: station === s ? null : s, byHand: true });
  return (
    <section className="hx-flow-section hx-flow-run-panel" data-testid="flow-run-panel" data-run={view.record.id} data-station={station ?? undefined}>
      <RunHead view={view} module={module} />
      {isDryRun(run) ? <DryControls module={module} run={run} status={view.record.status} /> : null}
      {def ? <FlowGraph def={def} view={view} selected={station} onSelect={pick} /> : null}
      {def && station ? <StationPanel key={`${run}/${station}`} station={station} def={def} view={view} module={module} initialTab="run" /> : null}
      <RunStations view={view} selected={station} onPick={pick} />
      <OutputDirLine view={view} onOpenFiles={onOpenFiles} />
      {isDryRun(run) ? null : <RunSessionsList module={module} run={run} rev={view.record.rev} />}
    </section>
  );
}
