/**
 * The flows inside another space — the system space (`registerSystemFlows`): its page lists them,
 * and a flow clicked there makes the left column that flow's.
 *
 *   on the page     the flows and every run (`FlowsTab`, `RunsTab`); a row clicked enters its flow's
 *                   context, the run shown if it was one                       (`SystemFlows`)
 *   left column     back to the space it is inside; the flow's name; its conversation; its runs and
 *                   dry runs. The general conversation's context when no flow is (a new flow is
 *                   written there)                                             (`FlowContext`)
 *
 * The rest is the flow space's own: the conversation in the middle, the flow's or the run's details
 * beside it, the reference in the right column.
 */

import { useMemo } from "react";
import { useT } from "@harnix/react";

import { useDefs, useDryRuns, useTopic } from "./hooks";
import { formatTime, isDryRun, phaseOf, runsOf, runsTopic } from "./model";
import type { FlowSpaceState } from "./state";
import { FlowsTab, PhaseBadge, RunsTab, useSelection, type FlowTabs } from "./tabs";

interface Props {
  state: FlowSpaceState;
  tabs: FlowTabs;
}

/** The page's part: the flows, then every run; a click goes into the flow. */
export function SystemFlows({ state, tabs }: Props) {
  const t = useT();
  return (
    <div className="hx-flow-system" data-testid="system-flows">
      <FlowsTab state={state} tabs={tabs} onPick={(flow) => void state.enter(flow)} />
      <div className="hx-flow-section-head hx-flow-system-runs">{t("flowui.tab.runs")}</div>
      <RunsTab state={state} tabs={tabs} onPick={(run, flow) => void state.enter(flow || null, run)} />
    </div>
  );
}

/** The left column inside a flow: the way back, the flow, its conversation and its runs. */
export function FlowContext({ state }: { state: FlowSpaceState }) {
  const t = useT();
  const sel = useSelection(state);
  const defs = useDefs(state.module);
  const runs = runsOf(useTopic<unknown>(runsTopic(state.module), {}));
  const dry = useDryRuns(state.module);
  const flow = sel.flow;
  const tab = state.tabNow();
  const rows = useMemo(() => {
    if (!flow) return [];
    return [
      ...runs.filter((r) => r.flow === flow).map((r) => ({ id: r.id, title: r.title || r.subject || r.id, phase: phaseOf(r), at: Number(r.created_at ?? 0), dry: !!r.dry || isDryRun(r.id) })),
      ...dry.filter((d) => d.flow === flow).map((d) => ({ id: d.id, title: d.id, phase: phaseOf(d), at: Number(d.created_at ?? 0), dry: true })),
    ].sort((a, b) => b.at - a.at || b.id.localeCompare(a.id));
  }, [flow, runs, dry]);
  const title = flow ? defs.list.find((d) => d.name === flow)?.title || flow : t("flowui.context.general");
  const own = tab === state.tabs.flows;
  return (
    <div className="hx-flow-context" data-testid="flow-context" data-flow={flow ?? ""}>
      <button type="button" className="hx-flow-back" data-testid="flow-back" onClick={() => void state.leave()}>
        <span aria-hidden="true">←</span> {t("flowui.context.back")}
      </button>
      <div className="hx-flow-context-head">
        <div className="hx-flow-context-title" data-testid="flow-context-title" title={title}>
          {title}
        </div>
        {flow ? <code className="hx-flow-id">{flow}</code> : null}
      </div>
      <div className="hx-flow-section-head">{t("flowui.context.conversation")}</div>
      <ul className="hx-flow-rows">
        <li>
          <button
            type="button"
            className={`hx-flow-row${own ? " on" : ""}`}
            data-testid="flow-context-conversation"
            aria-current={own ? "true" : undefined}
            onClick={() => state.selectFlow(flow)}
          >
            <span className="hx-flow-row-title">{t(flow ? "flowui.context.own" : "flowui.context.general")}</span>
          </button>
        </li>
      </ul>
      {flow ? (
        <>
          <div className="hx-flow-section-head">{t("flowui.tab.runs")}</div>
          {rows.length ? null : <p className="hx-flow-hint">{t("flowui.list.noRuns")}</p>}
          <ul className="hx-flow-rows" data-testid="flow-context-runs">
            {rows.map((r) => {
              const on = tab === state.tabs.runs && sel.run === r.id;
              return (
                <li key={r.id}>
                  <button
                    type="button"
                    className={`hx-flow-row${on ? " on" : ""}`}
                    data-testid={`flow-run-${r.id}`}
                    data-phase={r.phase.state}
                    data-dry={r.dry ? "true" : undefined}
                    aria-current={on ? "true" : undefined}
                    onClick={() => state.showRun(r.id)}
                  >
                    <span className="hx-flow-row-title">
                      <PhaseBadge phase={r.phase} />
                      {r.title}
                      {r.dry ? <span className="hx-flow-badge">{t("flowui.dry.badge")}</span> : null}
                    </span>
                    <span className="hx-flow-faint">{formatTime(r.at)}</span>
                  </button>
                </li>
              );
            })}
          </ul>
        </>
      ) : null}
    </div>
  );
}
