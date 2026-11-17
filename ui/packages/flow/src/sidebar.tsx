/**
 * The flow space's left column (`FlowSidebar`): what there is to talk to, and where the person is.
 *
 *   the way back    only while a task was entered from another space (the projects space)
 *   conversation    the general conversation, where flows are written and questions asked
 *   flows           every flow, the selected one open on its own conversation and its runs and dry
 *                   runs under it; a flow clicked is selected and its conversation opened, a run
 *                   clicked is shown with its main session in the middle
 *
 * A pick here brings the right column's list to the front with it (`FlowSpaceState.openList`), so
 * the two columns say the same thing; the right column's lists filter and sort, this one only lists.
 */

import { useMemo } from "react";
import { useT } from "@harnix/react";

import { useDefs, useDryRuns, useTopic } from "./hooks";
import { formatTime, isDryRun, phaseOf, runsOf, runsTopic } from "./model";
import type { FlowSpaceState } from "./state";
import { PhaseBadge, useSelection } from "./tabs";

export function FlowSidebar({ state }: { state: FlowSpaceState }) {
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
  const general = !flow && (tab === state.tabs.flows || tab === state.tabs.reference || (tab === state.tabs.runs && !sel.run));
  const from = state.from();
  const pickFlow = (name: string) => {
    state.selectFlow(name);
    state.openList(state.tabs.flows);
  };
  const pickRun = (id: string) => {
    state.showRun(id);
    state.openList(state.tabs.runs);
  };
  return (
    <div className="hx-flow-context" data-testid="flow-context" data-flow={flow ?? ""}>
      {from ? (
        <button type="button" className="hx-flow-sidebar-back" data-testid="flow-back" data-to={from.space} onClick={() => void state.leave()}>
          <span aria-hidden="true">←</span> {t("flowui.context.backTo", { space: from.title })}
        </button>
      ) : null}
      <div className="hx-flow-section-head">{t("flowui.context.conversation")}</div>
      <ul className="hx-flow-rows">
        <li>
          <button
            type="button"
            className={`hx-flow-row${general ? " on" : ""}`}
            data-testid="flow-context-conversation"
            aria-current={general ? "true" : undefined}
            onClick={() => {
              void state.openGeneral();
              state.openList(state.tabs.flows);
            }}
          >
            <span className="hx-flow-row-title">{t("flowui.context.general")}</span>
          </button>
        </li>
      </ul>
      <div className="hx-flow-section-head">{t("flowui.tab.flows")}</div>
      {defs.list.length ? null : <p className="hx-flow-hint">{t("flowui.list.noFlows")}</p>}
      <ul className="hx-flow-rows" data-testid="flow-context-flows">
        {defs.list.map((d) => {
          const on = flow === d.name;
          return (
            <li key={d.name}>
              <button
                type="button"
                className={`hx-flow-row${on && tab === state.tabs.flows ? " on" : ""}`}
                data-testid={`flow-context-flow-${d.name}`}
                aria-current={on ? "true" : undefined}
                data-valid={d.valid ? "true" : "false"}
                title={d.name}
                onClick={() => pickFlow(d.name)}
              >
                <span className="hx-flow-row-title">{d.title || d.name}</span>
              </button>
              {on ? (
                <ul className="hx-flow-rows hx-flow-context-runs" data-testid="flow-context-runs">
                  {rows.length ? null : (
                    <li>
                      <p className="hx-flow-hint">{t("flowui.list.noRuns")}</p>
                    </li>
                  )}
                  {rows.map((r) => {
                    const current = tab === state.tabs.runs && sel.run === r.id;
                    return (
                      <li key={r.id}>
                        <button
                          type="button"
                          className={`hx-flow-row${current ? " on" : ""}`}
                          data-testid={`flow-run-${r.id}`}
                          data-phase={r.phase.state}
                          data-dry={r.dry ? "true" : undefined}
                          aria-current={current ? "true" : undefined}
                          onClick={() => pickRun(r.id)}
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
              ) : null}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
