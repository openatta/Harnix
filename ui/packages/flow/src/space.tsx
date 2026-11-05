/**
 * `registerFlowSpace()`: the flow module's components put together as one space (flow-design §17,
 * "调试用的流程 space"), for an application that wants it.
 *
 *   left    the flows and their runs, and a form to start one       (`sidebar`)
 *   centre  the graph: a flow's definition, or a run over it         (`overview`, `center: "hidden"`)
 *   right   the station picked in the graph; the reference          (`tabs`)
 *
 * Also the `flow.block` that draws a session's proposal, and the `flowui` strings.
 */

import { useEffect, useState, useSyncExternalStore } from "react";
import { t as translate, useNest, useT, type Registry } from "@harnix/react";

import { FlowGraph } from "./FlowGraph";
import { FlowReference } from "./FlowReference";
import { errorText, useMethod, useRunView } from "./hooks";
import { defOf, formatTime, methodName, proposalOf, runTone, type BlockShape } from "./model";
import { ProposalCard } from "./ProposalCard";
import { RunList } from "./RunList";
import { StationPanel } from "./StationPanel";
import { registerFlowStrings, STRINGS_OWNER } from "./strings";
import type { DefInfo, RunView } from "./types";

export interface FlowSpaceOptions {
  /** The space's id; "flow" by default. */
  space?: string;
  /** The id the application registered the flow module under; "flow" by default (`app.flow.*`). */
  module?: string;
  /** The space's title; the `flowui.space` string by default. */
  title?: string;
  icon?: string;
  order?: number;
  group?: string;
  /** The owner the contributions are registered as; "flowui" by default. */
  owner?: string;
}

export interface FlowSelection {
  flow: string | null;
  run: string | null;
  station: string | null;
}

/** What the space shows, shared by its three columns. */
export class FlowSpaceState {
  private value: FlowSelection = { flow: null, run: null, station: null };
  private readonly listeners = new Set<() => void>();

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  readonly get = (): FlowSelection => this.value;

  /** A flow's definition, no run. */
  showFlow(flow: string): void {
    this.set({ flow, run: null, station: this.value.flow === flow && !this.value.run ? this.value.station : null });
  }

  showRun(run: string): void {
    // The station stays picked: the same station of another run, or of the flow just started.
    this.set({ flow: this.value.flow, run, station: this.value.station });
  }

  /** The run's flow, once known: the definitions list highlights it. */
  knowFlow(flow: string): void {
    if (this.value.flow !== flow) this.set({ ...this.value, flow });
  }

  pick(station: string | null): void {
    this.set({ ...this.value, station });
  }

  private set(next: FlowSelection): void {
    this.value = next;
    for (const fn of [...this.listeners]) fn();
  }
}

function useSelection(state: FlowSpaceState): FlowSelection {
  return useSyncExternalStore(state.subscribe, state.get);
}

export interface FlowSpace {
  state: FlowSpaceState;
  space: string;
  module: string;
  /** The right-column tabs' ids. */
  tabs: { station: string; reference: string };
}

export function registerFlowSpace(registry: Registry, options: FlowSpaceOptions = {}): FlowSpace {
  const space = options.space ?? "flow";
  const module = options.module ?? "flow";
  const owner = options.owner ?? STRINGS_OWNER;
  const state = new FlowSpaceState();
  const tabs = { station: `${space}.station`, reference: `${space}.reference` };

  registerFlowStrings(registry);

  registry.register(
    "space",
    {
      id: space,
      icon: options.icon ?? "fork",
      order: options.order ?? 60,
      ...(options.group !== undefined ? { group: options.group } : {}),
      title: () => options.title ?? translate("flowui.space"),
      subtitle: () => translate("flowui.space.subtitle"),
      center: "hidden",
      newSession: false,
      details: "open",
      detailsWidth: 520,
      sidebar: () => <Sidebar state={state} module={module} />,
      overview: () => <Overview state={state} module={module} stationTab={tabs.station} />,
      tabs: [
        { id: tabs.station, title: () => translate("flowui.tab.station"), render: () => <StationTab state={state} module={module} /> },
        { id: tabs.reference, title: () => translate("flowui.tab.reference"), render: () => <ReferenceTab state={state} module={module} />, align: "end" },
      ],
    },
    owner,
  );

  registry.register(
    "flow.block",
    {
      id: `${space}.proposal`,
      match: (block: BlockShape) => proposalOf(block, `app.${module}`) !== null,
      render: (block: BlockShape) => {
        const found = proposalOf(block, `app.${module}`)!;
        return <ProposalCard id={found.id} proposal={found.proposal} module={module} />;
      },
    },
    owner,
  );

  return { state, space, module, tabs };
}

/* ── the three columns ───────────────────────────────────────────────── */

function Sidebar({ state, module }: { state: FlowSpaceState; module: string }) {
  const sel = useSelection(state);
  return (
    <RunList
      module={module}
      selected={sel.run}
      flow={sel.flow}
      onSelect={(run) => state.showRun(run)}
      onSelectFlow={(name) => state.showFlow(name)}
    />
  );
}

/** A flow's definition with what `def` adds (skill bodies), or null while it loads. */
function useDefInfo(module: string, flow: string | null) {
  return useMethod<DefInfo>(module, "def", flow ? { name: flow } : null);
}

function Overview({ state, module, stationTab }: { state: FlowSpaceState; module: string; stationTab: string }) {
  const t = useT();
  const nest = useNest();
  const sel = useSelection(state);
  const view = useRunView(module, sel.run);
  const info = useDefInfo(module, sel.run ? null : sel.flow);
  const defs = useMethod<{ name: string }[] | { defs?: { name: string }[] }>(module, "defs", sel.flow || sel.run ? null : {});

  // Nothing chosen yet: the first flow. A run shown: its flow is the one highlighted.
  const first = (Array.isArray(defs.value) ? defs.value[0] : defs.value?.defs?.[0])?.name;
  useEffect(() => {
    if (first && !state.get().flow && !state.get().run) state.showFlow(first);
  }, [state, first]);
  const runFlow = view?.record.flow;
  useEffect(() => {
    if (runFlow) state.knowFlow(runFlow);
  }, [state, runFlow]);

  const pick = (station: string) => {
    state.pick(station);
    nest.details.open(stationTab);
  };

  if (sel.run) {
    if (!view) return <div className="hx-flow-overview"><p className="hx-flow-hint">{t("flowui.loading")}</p></div>;
    return (
      <div className="hx-flow-overview" data-testid="flow-overview" data-run={view.record.id}>
        <RunHeader view={view} module={module} onBack={() => state.showFlow(view.record.flow)} />
        <FlowGraph def={defOf(view)!} view={view} selected={sel.station} onSelect={pick} />
      </div>
    );
  }
  if (sel.flow) {
    const def = info.value?.def;
    return (
      <div className="hx-flow-overview" data-testid="flow-overview" data-flow={sel.flow}>
        <div className="hx-flow-head">
          <div className="hx-flow-head-title">{def?.title || sel.flow}</div>
          <code className="hx-flow-id">{sel.flow}</code>
          {info.value?.digest ? <code className="hx-flow-id hx-flow-faint" title={info.value.digest}>{info.value.digest.slice(0, 12)}</code> : null}
        </div>
        {def?.description ? <p className="hx-flow-desc">{def.description}</p> : null}
        {info.error ? <div className="hx-flow-error">{info.error}</div> : null}
        {def ? <FlowGraph def={def} selected={sel.station} onSelect={pick} /> : info.error ? null : <p className="hx-flow-hint">{t("flowui.loading")}</p>}
      </div>
    );
  }
  return (
    <div className="hx-flow-overview" data-testid="flow-overview">
      <p className="hx-flow-hint">{defs.error ?? t("flowui.overview.empty")}</p>
    </div>
  );
}

function RunHeader({ view, module, onBack }: { view: RunView; module: string; onBack(): void }) {
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
        <button type="button" className="btn sm ghost" data-testid="flow-back" onClick={onBack} title={t("flowui.overview.back")}>
          ←
        </button>
        <div className="hx-flow-head-title">{run.title || run.subject}</div>
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

function StationTab({ state, module }: { state: FlowSpaceState; module: string }) {
  const t = useT();
  const sel = useSelection(state);
  const view = useRunView(module, sel.run);
  const flow = sel.run ? (view?.record.flow ?? null) : sel.flow;
  const info = useDefInfo(module, flow);
  const def = sel.run ? defOf(view) : info.value?.def;
  if (!sel.station || !def) {
    return (
      <div className="hx-flow-panel" data-testid="flow-station-panel">
        <p className="hx-flow-hint">{t("flowui.panel.pick")}</p>
      </div>
    );
  }
  return <StationPanel key={`${sel.run ?? ""}/${sel.station}`} station={sel.station} def={def} view={sel.run ? view : null} info={info.value ?? null} module={module} />;
}

function ReferenceTab({ state, module }: { state: FlowSpaceState; module: string }) {
  const sel = useSelection(state);
  const info = useDefInfo(module, sel.flow);
  return <FlowReference module={module} initialYaml={info.value?.yaml ?? ""} />;
}
