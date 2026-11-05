/**
 * The flow space's right column (xops-02 §7): it shows, it does not edit a definition — that is
 * done in the conversation, by the Agent's tools.
 *
 *   流程 / Flows      the flows (title, origin, valid); the selected one's graph, skills and dry runs
 *   运行 / Runs       the selected flow's runs and dry runs; the selected run's graph, its stations'
 *                     details and the hands to act on them; a dry run's step / run-to-the-end
 *   参考 / Reference  the definition's fields and this application's catalog (`FlowReference`)
 *
 * And the line over the composer that names the conversation's current flow.
 */

import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { useNest, useT, type HostSessionRef as SessionRef } from "@harnix/react";

import { FlowGraph } from "./FlowGraph";
import { FlowReference } from "./FlowReference";
import { errorText, useDefs, useDryRuns, useMethod, useRunView, useTopic } from "./hooks";
import { defOf, formatTime, isDryRun, methodName, openOf, problemText, runIdOf, runTone, runsOf, runsTopic } from "./model";
import { StartForm } from "./RunList";
import type { FlowSelection, FlowSpaceState } from "./state";
import { StationPanel } from "./StationPanel";
import type { DefInfo, DefSummary, DryProgress, RunStatus, RunView } from "./types";

/** The right column's tab ids. */
export interface FlowTabs {
  flows: string;
  runs: string;
  reference: string;
}

interface TabProps {
  state: FlowSpaceState;
  tabs: FlowTabs;
}

export function useSelection(state: FlowSpaceState): FlowSelection {
  const nest = useNest();
  useEffect(() => state.attach(nest), [state, nest]);
  return useSyncExternalStore(state.subscribe, state.get);
}

function titleOf(defs: DefSummary[], name: string): string {
  return defs.find((d) => d.name === name)?.title || name;
}

/* ── the line over the composer ──────────────────────────────────────── */

/**
 * The conversation's current flow, or how to choose one. Never a block: creating a flow needs no
 * selection, and the Agent says so when a tool needs one.
 */
export function FocusNotice({ state, session }: { state: FlowSpaceState; session: SessionRef }) {
  const t = useT();
  const sel = useSelection(state);
  const { list } = useDefs(state.module);
  const mine = state.session();
  const remembered = mine && mine.instance === session.instance && mine.sessionId === session.sessionId ? state.focusOf() : undefined;
  const flow = remembered === undefined ? sel.flow : remembered;
  if (!flow) {
    return (
      <span data-testid="flow-focus-notice" data-flow="">
        {t("flowui.focus.none")}
      </span>
    );
  }
  return (
    <span data-testid="flow-focus-notice" data-flow={flow}>
      {t("flowui.focus.current", { title: titleOf(list, flow) })} <code className="hx-flow-id">{flow}</code>
    </span>
  );
}

/* ── 流程 ────────────────────────────────────────────────────────────── */

export function FlowsTab({ state, tabs }: TabProps) {
  const t = useT();
  const sel = useSelection(state);
  const defs = useDefs(state.module);
  return (
    <div className="hx-flow-panel" data-testid="flow-flows">
      <section className="hx-flow-section" data-testid="flow-defs">
        <div className="hx-flow-section-head">{t("flowui.list.flows")}</div>
        {defs.error ? <div className="hx-flow-error">{defs.error}</div> : null}
        {defs.loaded && !defs.list.length ? <p className="hx-flow-hint">{t("flowui.list.noFlows")}</p> : null}
        <ul className="hx-flow-rows">
          {defs.list.map((d) => (
            <li key={d.name}>
              <button
                type="button"
                className={`hx-flow-row${sel.flow === d.name ? " on" : ""}`}
                data-testid={`flow-def-${d.name}`}
                data-origin={d.origin ?? undefined}
                data-valid={d.valid === false ? "false" : "true"}
                aria-current={sel.flow === d.name ? "true" : undefined}
                onClick={() => state.selectFlow(d.name)}
              >
                <span className="hx-flow-row-title">
                  {d.title || d.name}
                  {d.origin ? <span className="hx-flow-badge">{t("flowui.origin." + d.origin)}</span> : null}
                  {d.valid === false ? <span className="hx-flow-badge hx-flow-tone-error">{t("flowui.def.invalid")}</span> : null}
                </span>
                <span className="hx-flow-faint">{d.name}</span>
              </button>
            </li>
          ))}
        </ul>
      </section>
      {sel.flow ? (
        <FlowDetail key={sel.flow} state={state} tabs={tabs} flow={sel.flow} summary={defs.list.find((d) => d.name === sel.flow)} station={sel.station} />
      ) : (
        <p className="hx-flow-hint" data-testid="flow-pick-hint">
          {t("flowui.flows.pick")}
        </p>
      )}
    </div>
  );
}

function FlowDetail({ state, tabs, flow, summary, station }: TabProps & { flow: string; summary: DefSummary | undefined; station: string | null }) {
  const t = useT();
  // Read again whenever the list says the flow changed: the conversation saved it.
  const info = useMethod<DefInfo>(state.module, "def", { name: flow }, summary?.digest);
  const def = info.value?.def;
  const digest = info.value?.digest ?? summary?.digest ?? "";
  const skills = Object.entries(info.value?.skills ?? {});
  const problems = info.value?.problems ?? [];
  return (
    <section className="hx-flow-section" data-testid="flow-detail" data-flow={flow} data-digest={digest}>
      <div className="hx-flow-head">
        <div className="hx-flow-head-title" data-testid="flow-detail-title">
          {def?.title || summary?.title || flow}
        </div>
        <code className="hx-flow-id">{flow}</code>
        {digest ? (
          <code className="hx-flow-id hx-flow-faint" title={digest}>
            {digest.slice(0, 12)}
          </code>
        ) : null}
      </div>
      {def?.description ? <p className="hx-flow-desc">{def.description}</p> : null}
      {info.error ? <div className="hx-flow-error">{info.error}</div> : null}
      {problems.length ? (
        <ul className="hx-flow-list hx-flow-problems" data-testid="flow-def-problems">
          {problems.map((p, i) => (
            <li key={i} className="hx-flow-tone-error">
              {problemText(p)}
            </li>
          ))}
        </ul>
      ) : null}
      {def ? (
        <FlowGraph def={def} selected={station} onSelect={(s) => state.pick(station === s ? null : s)} />
      ) : info.error ? null : (
        <p className="hx-flow-hint">{t("flowui.loading")}</p>
      )}
      {def && station ? <StationPanel key={station} station={station} def={def} info={info.value ?? null} module={state.module} initialTab="definition" /> : null}
      <div className="hx-flow-section-head">{t("flowui.flows.skills")}</div>
      {skills.length ? (
        <div data-testid="flow-skills">
          {skills.map(([name, body]) => (
            <details key={name} className="hx-flow-details" data-testid={`flow-skill-body-${name}`}>
              <summary>{t("flowui.def.skill", { name })}</summary>
              <pre className="hx-flow-code hx-flow-prose">{body}</pre>
            </details>
          ))}
        </div>
      ) : (
        <p className="hx-flow-hint">{t("flowui.flows.noSkills")}</p>
      )}
      <DryRuns state={state} tabs={tabs} flow={flow} />
    </section>
  );
}

function DryRuns({ state, tabs, flow }: TabProps & { flow: string }) {
  const t = useT();
  const nest = useNest();
  const mine = useDryRuns(state.module).filter((d) => d.flow === flow);
  const start = useDryStart(state, tabs);
  return (
    <>
      <div className="hx-flow-section-head">
        <span>{t("flowui.flows.dryRuns")}</span>
        <span className="hx-flow-grow" />
        <button type="button" className="btn sm outline" data-testid="flow-dry-start" disabled={start.busy} onClick={() => void start.run(flow)}>
          {t("flowui.dry.start")}
        </button>
      </div>
      {start.error ? <div className="hx-flow-error">{start.error}</div> : null}
      {mine.length ? (
        <ul className="hx-flow-rows" data-testid="flow-dry-runs">
          {mine.map((d) => (
            <li key={d.id}>
              <button
                type="button"
                className="hx-flow-row"
                data-testid={`flow-dry-${d.id}`}
                data-status={d.status}
                onClick={() => {
                  state.showRun(d.id);
                  nest.details.open(tabs.runs);
                }}
              >
                <span className="hx-flow-row-title">
                  <span className={`hx-flow-dot hx-flow-tone-${runTone(d.status)}`} aria-hidden="true" />
                  {d.id}
                </span>
                <span className="hx-flow-faint">
                  {t("flowui.runStatus." + d.status)} · {formatTime(d.created_at)}
                </span>
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="hx-flow-hint">{t("flowui.flows.noDryRuns")}</p>
      )}
    </>
  );
}

/** Start a dry run of a flow and show it in the runs tab. */
function useDryStart(state: FlowSpaceState, tabs: FlowTabs) {
  const nest = useNest();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const run = async (flow: string) => {
    setBusy(true);
    setError(null);
    try {
      const id = runIdOf(await nest.call(methodName(state.module, "dry_run.start"), { flow }));
      if (id) {
        state.showRun(id);
        nest.details.open(tabs.runs);
      }
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return { busy, error, run };
}

/* ── 运行 ────────────────────────────────────────────────────────────── */

interface Row {
  id: string;
  flow: string;
  title: string;
  status: RunStatus;
  created_at: number;
  open: string[];
  dry: boolean;
}

export function RunsTab({ state, tabs }: TabProps) {
  const t = useT();
  const sel = useSelection(state);
  const defs = useDefs(state.module);
  const runs = runsOf(useTopic<unknown>(runsTopic(state.module), {}));
  const dry = useDryRuns(state.module);
  const [starting, setStarting] = useState(false);
  const start = useDryStart(state, tabs);

  const rows = useMemo<Row[]>(() => {
    const all: Row[] = [
      ...runs.map((r) => ({ id: r.id, flow: r.flow, title: r.title || r.subject || r.id, status: r.status, created_at: Number(r.created_at ?? 0), open: openOf(r), dry: false })),
      ...dry.map((d) => ({ id: d.id, flow: d.flow, title: d.id, status: d.status, created_at: Number(d.created_at ?? 0), open: [], dry: true })),
    ];
    return all
      .filter((r) => !sel.flow || r.flow === sel.flow)
      .sort((a, b) => b.created_at - a.created_at || b.id.localeCompare(a.id));
  }, [runs, dry, sel.flow]);

  return (
    <div className="hx-flow-panel" data-testid="flow-runs">
      <section className="hx-flow-section">
        <div className="hx-flow-section-head">
          <span>{sel.flow ? t("flowui.runs.of", { flow: titleOf(defs.list, sel.flow) }) : t("flowui.runs.all")}</span>
          <span className="hx-flow-grow" />
          <button type="button" className="btn sm outline" data-testid="flow-start-open" aria-expanded={starting} onClick={() => setStarting(!starting)}>
            {t("flowui.start.open")}
          </button>
          <button type="button" className="btn sm outline" data-testid="flow-runs-dry-start" disabled={!sel.flow || start.busy} onClick={() => sel.flow && void start.run(sel.flow)}>
            {t("flowui.dry.start")}
          </button>
        </div>
        {start.error ? <div className="hx-flow-error">{start.error}</div> : null}
        {starting ? (
          <StartForm
            key={sel.flow ?? ""}
            module={state.module}
            defs={defs.list}
            initial={sel.flow ?? defs.list[0]?.name ?? ""}
            onStarted={(run) => {
              setStarting(false);
              if (run) state.showRun(run);
            }}
          />
        ) : null}
        {rows.length === 0 ? <p className="hx-flow-hint">{t("flowui.list.noRuns")}</p> : null}
        <ul className="hx-flow-rows" data-testid="flow-run-list">
          {rows.map((r) => (
            <li key={r.id}>
              <button
                type="button"
                className={`hx-flow-row${sel.run === r.id ? " on" : ""}`}
                data-testid={`flow-run-${r.id}`}
                data-status={r.status}
                data-dry={r.dry ? "true" : undefined}
                aria-current={sel.run === r.id ? "true" : undefined}
                onClick={() => state.showRun(r.id)}
              >
                <span className="hx-flow-row-title">
                  <span className={`hx-flow-dot hx-flow-tone-${runTone(r.status)}`} aria-hidden="true" />
                  {r.title}
                  {r.dry ? <span className="hx-flow-badge">{t("flowui.dry.badge")}</span> : null}
                </span>
                <span className="hx-flow-faint">
                  {r.flow} · {t("flowui.runStatus." + r.status)}
                  {r.open.length ? ` · ${r.open.join(", ")}` : ""} · {formatTime(r.created_at)}
                </span>
              </button>
            </li>
          ))}
        </ul>
      </section>
      {sel.run ? <RunDetail key={sel.run} state={state} run={sel.run} station={sel.station} /> : <p className="hx-flow-hint">{t("flowui.runs.pick")}</p>}
    </div>
  );
}

function RunDetail({ state, run, station }: { state: FlowSpaceState; run: string; station: string | null }) {
  const t = useT();
  const view = useRunView(state.module, run);
  if (!view) return <p className="hx-flow-hint">{t("flowui.loading")}</p>;
  const def = defOf(view);
  return (
    <section className="hx-flow-section" data-testid="flow-run-detail" data-run={view.record.id}>
      <RunHead view={view} module={state.module} />
      {isDryRun(run) ? <DryControls module={state.module} run={run} status={view.record.status} /> : null}
      {def ? <FlowGraph def={def} view={view} selected={station} onSelect={(s) => state.pick(station === s ? null : s)} /> : null}
      {def && station ? <StationPanel key={`${run}/${station}`} station={station} def={def} view={view} module={state.module} initialTab="run" /> : null}
    </section>
  );
}

function RunHead({ view, module }: { view: RunView; module: string }) {
  const t = useT();
  const nest = useNest();
  const run = view.record;
  const [cancelling, setCancelling] = useState(false);
  const [reason, setReason] = useState("");
  const [error, setError] = useState<string | null>(null);
  const cancel = async () => {
    setError(null);
    try {
      await nest.call(methodName(module, "cancel"), { run: run.id, reason: reason.trim() });
      setCancelling(false);
      setReason("");
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <>
      <div className="hx-flow-head">
        <div className="hx-flow-head-title">{run.title || run.subject || run.id}</div>
        <code className="hx-flow-id">{run.id}</code>
        <span className={`hx-flow-pill hx-flow-tone-${runTone(run.status)}`} data-testid="flow-run-status" data-status={run.status}>
          {t("flowui.runStatus." + run.status)}
        </span>
        <span className="hx-flow-grow" />
        <button type="button" className="btn sm outline" data-testid="flow-cancel-run" disabled={run.status !== "active"} onClick={() => setCancelling(!cancelling)}>
          {t("flowui.act.cancelRun")}
        </button>
      </div>
      <div className="hx-flow-meta hx-flow-faint">
        {t("flowui.overview.meta", { flow: run.flow, subject: run.subject, at: formatTime(run.created_at), by: `${run.created_by?.kind ?? ""}:${run.created_by?.id ?? ""}` })}
        {run.parent ? ` · ${t("flowui.overview.parent", { run: run.parent.run, station: run.parent.station })}` : ""}
      </div>
      {cancelling ? (
        <div className="hx-flow-form" data-testid="flow-cancel-form">
          <textarea className="hx-flow-input" data-testid="flow-cancel-reason" rows={2} value={reason} placeholder={t("flowui.act.reasonRequired")} onChange={(e) => setReason(e.target.value)} />
          <div className="hx-flow-actions">
            <button type="button" className="btn sm primary" data-testid="flow-cancel-submit" disabled={!reason.trim()} onClick={() => void cancel()}>
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
  const nest = useNest();
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

/* ── 参考 ────────────────────────────────────────────────────────────── */

export function ReferenceTab({ state }: { state: FlowSpaceState }) {
  const sel = useSelection(state);
  const info = useMethod<DefInfo>(state.module, "def", sel.flow ? { name: sel.flow } : null);
  return <FlowReference module={state.module} initialYaml={info.value?.yaml ?? ""} />;
}
