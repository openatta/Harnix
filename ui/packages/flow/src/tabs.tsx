/**
 * The flow space's right column (xops-03 §6): it shows, it does not edit a definition — that is
 * done in the conversation, by the Agent's tools. What is selected here is what the middle talks to
 * (`FlowSpaceState`).
 *
 *   流程 / Flows      first row: add (the general conversation, the composer started) · filter, sort;
 *                     the flows (title, origin, valid)
 *   运行 / Runs       first row: filter (state, flow, kind), sort (start, duration, tokens); every
 *                     run and dry run
 *   参考 / Reference  the framework of a flow in one picture (`FlowFramework`), coloured by the
 *                     guide's five groups; the guide below it, in those groups (an element clicked
 *                     goes to its row); then this application's catalog and the fields
 *
 * What is selected opens in the shell's second side panel, beside the conversation (xops-04 §7,
 * `FlowSecondary`): a flow — its diagram (a station picked there opens below it), its definition
 * station by station, its 配套 (skills, rules, policies, gates, the hooks over it), its YAML, and a
 * dry run or a run of it started from there; a run — its graph with each station's time and tokens,
 * its stations' details and their hands, pause / resume / cancel, its sub-agents and tasks.
 *
 * And the line over the composer that names the conversation's flow, and what an empty
 * conversation says it is for.
 */

import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { useLocale, useNest, useT, type HostSessionRef as SessionRef } from "@harnix/react";

import { FlowFramework, type FrameworkPick } from "./FlowFramework";
import { FlowGraph } from "./FlowGraph";
import { FlowReference } from "./FlowReference";
import { errorText, useDefs, useDryRuns, useMethod, useRunView, useTopic } from "./hooks";
import {
  MAIN_SESSION_KEY,
  PHASE_STATES,
  defOf,
  formatDuration,
  gateTitle,
  formatTime,
  formatTokens,
  guideBlocks,
  guideSections,
  guideTermOf,
  inlineOf,
  isDryRun,
  methodName,
  phaseOf,
  phaseTone,
  permissionsOf,
  problemText,
  rulesOf,
  runElapsed,
  runIdOf,
  runUsage,
  runsOf,
  runsTopic,
  sessionOf,
  stationTitle,
  tokensOf,
  type GuideBlock,
  type GuideGroup,
} from "./model";
import { FlowParts, PermissionList, RuleChips } from "./parts";
import { StartForm } from "./RunList";
import type { FlowSelection, FlowSpaceState } from "./state";
import { executorText, StationPanel, stationPermissions } from "./StationPanel";
import type { DefInfo, DefSummary, DryProgress, FlowDef, Guide, Phase, RunSessions, RunStatus, RunView, SessionWire, Station } from "./types";

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

/** A list drawn on another space's page (the system console): a row clicked goes there with it. */
interface ListProps extends TabProps {
  /** Given: a click calls it with the row instead of selecting (or letting go of) it here. */
  onPick?: (id: string, flow: string) => void;
}

export function useSelection(state: FlowSpaceState): FlowSelection {
  const nest = useNest();
  useEffect(() => {
    state.attach(nest);
    // Drawn: the space is in front, whether or not a session change said so.
    state.lookFront();
  }, [state, nest]);
  return useSyncExternalStore(state.subscribe, state.get);
}

function titleOf(defs: DefSummary[], name: string): string {
  return defs.find((d) => d.name === name)?.title || name;
}

/** Now, again every second while `live`: elapsed times of what is still going. */
function useNow(live: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!live) return undefined;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [live]);
  return now;
}

/** Open the general conversation and start its composer with `text`. */
async function startInGeneral(state: FlowSpaceState, nest: ReturnType<typeof useNest>, text: string): Promise<void> {
  const ref = await state.openGeneral();
  if (ref) nest.composer.insert(text, { session: ref });
}

function asRef(s: SessionWire | null | undefined): SessionRef | null {
  return s && typeof s.instance === "string" && typeof s.session_id === "string" ? { instance: s.instance, sessionId: s.session_id } : null;
}

/* ── the line over the composer ──────────────────────────────────────── */

/**
 * The conversation's flow, or that it is the general one. Never a block: creating a flow needs no
 * selection, and the Agent says so when a tool needs one.
 */
export function FocusNotice({ state, session }: { state: FlowSpaceState; session: SessionRef }) {
  const t = useT();
  useSelection(state);
  const { list } = useDefs(state.module);
  const mine = state.session();
  const flow = mine && mine.instance === session.instance && mine.sessionId === session.sessionId ? (state.focusOf() ?? null) : null;
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

type FlowSort = "name" | "title";

export function FlowsTab({ state, tabs, onPick }: ListProps) {
  const t = useT();
  const nest = useNest();
  const sel = useSelection(state);
  const defs = useDefs(state.module);
  const [origin, setOrigin] = useState("");
  const [valid, setValid] = useState("");
  const [text, setText] = useState("");
  const [sort, setSort] = useState<FlowSort>("name");
  const shown = useMemo(() => {
    const q = text.trim().toLowerCase();
    return defs.list
      .filter((d) => !origin || (d.origin ?? "builtin") === origin)
      .filter((d) => !valid || (valid === "valid" ? d.valid !== false : d.valid === false))
      .filter((d) => !q || d.name.toLowerCase().includes(q) || (d.title ?? "").toLowerCase().includes(q))
      .sort((a, b) => (sort === "title" ? (a.title || a.name).localeCompare(b.title || b.name) : a.name.localeCompare(b.name)));
  }, [defs.list, origin, valid, text, sort]);
  return (
    <div className="hx-flow-panel" data-testid="flow-flows">
      <section className="hx-flow-section" data-testid="flow-defs">
        <div className="hx-flow-toolbar" data-testid="flow-flows-toolbar">
          <button type="button" className="btn sm primary" data-testid="flow-add" onClick={() => void startInGeneral(state, nest, t("flowui.add.prompt"))}>
            {t("flowui.add")}
          </button>
          <span className="hx-flow-grow" />
          <select aria-label={t("flowui.filter.origin")} data-testid="flow-filter-origin" value={origin} onChange={(e) => setOrigin(e.target.value)}>
            <option value="">{t("flowui.filter.allOrigins")}</option>
            {(["builtin", "override", "user"] as const).map((o) => (
              <option key={o} value={o}>
                {t("flowui.origin." + o)}
              </option>
            ))}
          </select>
          <select aria-label={t("flowui.filter.valid")} data-testid="flow-filter-valid" value={valid} onChange={(e) => setValid(e.target.value)}>
            <option value="">{t("flowui.filter.anyValid")}</option>
            <option value="valid">{t("flowui.filter.valid")}</option>
            <option value="invalid">{t("flowui.def.invalid")}</option>
          </select>
          <input
            className="hx-flow-input hx-flow-search"
            type="search"
            aria-label={t("flowui.filter.name")}
            placeholder={t("flowui.filter.name")}
            data-testid="flow-filter-name"
            value={text}
            onChange={(e) => setText(e.target.value)}
          />
          <select aria-label={t("flowui.sort")} data-testid="flow-sort-flows" value={sort} onChange={(e) => setSort(e.target.value as FlowSort)}>
            <option value="name">{t("flowui.sort.name")}</option>
            <option value="title">{t("flowui.sort.title")}</option>
          </select>
        </div>
        {defs.error ? <div className="hx-flow-error">{defs.error}</div> : null}
        {defs.loaded && !defs.list.length ? <p className="hx-flow-hint">{t("flowui.list.noFlows")}</p> : null}
        {defs.list.length && !shown.length ? <p className="hx-flow-hint">{t("flowui.list.noFlowMatch")}</p> : null}
        <ul className="hx-flow-rows" data-testid="flow-def-list">
          {shown.map((d) => (
            <li key={d.name}>
              <button
                type="button"
                className={`hx-flow-row${sel.flow === d.name ? " on" : ""}`}
                data-testid={`flow-def-${d.name}`}
                data-origin={d.origin ?? undefined}
                data-valid={d.valid === false ? "false" : "true"}
                aria-current={sel.flow === d.name ? "true" : undefined}
                onClick={() => (onPick ? onPick(d.name, d.name) : state.selectFlow(sel.flow === d.name ? null : d.name))}
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
      {sel.flow ? null : (
        <p className="hx-flow-hint" data-testid="flow-pick-hint">
          {t("flowui.flows.pick")}
        </p>
      )}
    </div>
  );
}

/* ── the second side panel ──────────────────────────────────────────── */

/** What the second side panel shows: the selected flow's details, or the selected run's. */
export function FlowSecondary({ state, tabs }: TabProps) {
  const sel = useSelection(state);
  const defs = useDefs(state.module);
  const detail = state.detail();
  if (detail?.kind === "flow") {
    return <FlowDetail key={detail.flow} state={state} tabs={tabs} flow={detail.flow} summary={defs.list.find((d) => d.name === detail.flow)} station={sel.station} />;
  }
  if (detail?.kind === "run") return <RunDetail key={detail.run} state={state} run={detail.run} station={sel.station} />;
  return null;
}

/** What an empty conversation of this space is for: a flow's own, a run's main session, the general one. */
export function EmptyConversation({ state, session }: { state: FlowSpaceState; session: SessionRef }) {
  const t = useT();
  const sel = useSelection(state);
  const { list } = useDefs(state.module);
  const mine = state.session();
  if (!mine || mine.instance !== session.instance || mine.sessionId !== session.sessionId) return null;
  const tab = state.tabNow();
  let kind: "run" | "flow" | "general" = "general";
  let name = "";
  if (tab === state.tabs.runs && sel.run) {
    kind = "run";
    name = sel.run;
  } else if (state.focusOf()) {
    kind = "flow";
    name = titleOf(list, state.focusOf()!);
  }
  return (
    <div data-testid="flow-empty-conversation" data-kind={kind}>
      <div className="hx-flow-empty-title">{t("flowui.empty." + kind + ".title", { name })}</div>
      <p>{t("flowui.empty." + kind)}</p>
      <ul className="hx-flow-list">
        {[1, 2, 3].map((i) => (
          <li key={i}>{t(`flowui.empty.${kind}.ask${i}`)}</li>
        ))}
      </ul>
    </div>
  );
}

/** One station, briefly, for a flow's definition: who does it, with what, under what, how it is judged. */
function StationSummary({ def, s, info, picked, onPick }: { def: FlowDef; s: Station; info: DefInfo | null; picked: boolean; onPick(): void }) {
  const t = useT();
  const specs = s.tasks?.length ? s.tasks : s.executor ? [{ id: s.id, executor: s.executor, skill: s.skill ?? null }] : [];
  const skills = [...new Set(specs.map((x) => x.skill).filter((x): x is string => !!x))];
  return (
    <li className={`hx-flow-station-card${picked ? " on" : ""}`} data-testid={`flow-def-station-${s.id}`}>
      <button type="button" className="hx-flow-station-card-head" aria-pressed={picked} onClick={onPick}>
        <strong>{stationTitle(s)}</strong> <code className="hx-flow-id">{s.id}</code>
      </button>
      <dl className="hx-flow-dl">
        <dt>{t("flowui.def.who")}</dt>
        <dd>{specs.length ? specs.map((x) => executorText(x.executor)).join("; ") : t("flowui.graph.gate")}</dd>
        {skills.length ? (
          <>
            <dt>{t("flowui.def.skills")}</dt>
            <dd>
              {skills.map((k) => (
                <code key={k} className="hx-flow-chip">
                  {k}
                </code>
              ))}
            </dd>
          </>
        ) : null}
        <dt>{t("flowui.def.rules")}</dt>
        <dd>
          <RuleChips rules={info?.stations?.[s.id]?.rules ?? rulesOf(def, s)} info={info} />
        </dd>
        <dt>{t("flowui.def.permissions")}</dt>
        <dd>
          <PermissionList permissions={stationPermissions(def, s, info, null)} testid={`flow-def-permissions-${s.id}`} />
        </dd>
        <dt>{t("flowui.def.exit")}</dt>
        <dd>
          {(s.exit ?? []).map((c, i) => (
            <span key={i} className="hx-flow-chip" data-kind={c.kind}>
              {t("flowui.exit." + c.kind)}
              {c.kind === "gate" ? ` ${c.gate}${gateTitle(c.gate, info) ? ` · ${gateTitle(c.gate, info)}` : ""}` : c.kind === "confirm" && c.who ? ` · ${c.who}` : ""}
            </span>
          ))}
        </dd>
        {s.rework_to?.length ? (
          <>
            <dt>{t("flowui.def.rework")}</dt>
            <dd>{s.rework_to.join(", ")}</dd>
          </>
        ) : null}
      </dl>
    </li>
  );
}

export function FlowDetail({ state, tabs, flow, summary, station }: TabProps & { flow: string; summary: DefSummary | undefined; station: string | null }) {
  const t = useT();
  const nest = useNest();
  // Read again whenever the list says the flow changed: the conversation saved it.
  const info = useMethod<DefInfo>(state.module, "def", { name: flow }, summary?.digest);
  const def = info.value?.def;
  const digest = info.value?.digest ?? summary?.digest ?? "";
  const problems = info.value?.problems ?? [];
  const dry = useDryStart(state, tabs);
  const [starting, setStarting] = useState(false);
  const defs = useDefs(state.module);
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
        <span className="hx-flow-grow" />
        <button type="button" className="btn sm outline" data-testid="flow-dry-start" disabled={dry.busy} onClick={() => void dry.run(flow)}>
          {t("flowui.dry.start")}
        </button>
        <button type="button" className="btn sm outline" data-testid="flow-start-open" aria-expanded={starting} disabled={summary?.valid === false} onClick={() => setStarting(!starting)}>
          {t("flowui.start.open")}
        </button>
      </div>
      {def?.description ? <p className="hx-flow-desc">{def.description}</p> : null}
      {dry.error ? <div className="hx-flow-error">{dry.error}</div> : null}
      {starting ? (
        <StartForm
          module={state.module}
          defs={defs.list.filter((d) => d.name === flow)}
          initial={flow}
          onStarted={(run) => {
            setStarting(false);
            if (run) {
              state.showRun(run);
              state.openList(tabs.runs);
            }
          }}
        />
      ) : null}
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
      <div className="hx-flow-section-head">{t("flowui.detail.diagram")}</div>
      {def ? (
        <div className="hx-flow-diagram" data-testid="flow-diagram">
          <FlowGraph def={def} selected={station} onSelect={(s) => state.pick(station === s ? null : s)} />
        </div>
      ) : info.error ? null : (
        <p className="hx-flow-hint">{t("flowui.loading")}</p>
      )}
      {def && station ? <StationPanel key={station} station={station} def={def} info={info.value ?? null} module={state.module} initialTab="definition" /> : null}
      <div className="hx-flow-section-head">{t("flowui.detail.definition")}</div>
      {def ? (
        <div data-testid="flow-definition-all">
          <dl className="hx-flow-dl" data-testid="flow-def-flow-level">
            <dt>{t("flowui.def.flowRules")}</dt>
            <dd>
              <RuleChips rules={def.rules ?? []} info={info.value ?? null} />
            </dd>
            <dt>{t("flowui.def.flowPolicies")}</dt>
            <dd>
              {def.policies?.length ? (
                def.policies.map((p) => (
                  <code key={p} className="hx-flow-chip">
                    {p}
                  </code>
                ))
              ) : (
                <span className="hx-flow-faint">{t("flowui.def.none")}</span>
              )}
            </dd>
            {def.permissions?.length ? (
              <>
                <dt>{t("flowui.def.flowPermissions")}</dt>
                <dd>
                  <PermissionList permissions={permissionsOf({ ...def, policies: [] }, { id: "", exit: [] }, null)} />
                </dd>
              </>
            ) : null}
          </dl>
          <ul className="hx-flow-station-cards">
            {def.stations.map((s) => (
              <StationSummary key={s.id} def={def} s={s} info={info.value ?? null} picked={station === s.id} onPick={() => state.pick(station === s.id ? null : s.id)} />
            ))}
          </ul>
        </div>
      ) : null}
      <div className="hx-flow-section-head">{t("flowui.detail.parts")}</div>
      <FlowParts module={state.module} info={info.value ?? null} flow={flow} />
      {info.value?.yaml ? (
        <details className="hx-flow-details" data-testid="flow-yaml">
          <summary>{t("flowui.flows.yaml")}</summary>
          <pre className="hx-flow-code hx-flow-mono" data-testid="flow-yaml-text">
            {info.value.yaml}
          </pre>
        </details>
      ) : null}
    </section>
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
        state.openList(tabs.runs);
        state.showRun(id);
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
  phase: Phase;
  created_at: number;
  elapsed: number | null;
  tokens: number;
  dry: boolean;
}

type RunSort = "start" | "duration" | "tokens";

export function RunsTab({ state, onPick }: ListProps) {
  const t = useT();
  const sel = useSelection(state);
  const defs = useDefs(state.module);
  const runs = runsOf(useTopic<unknown>(runsTopic(state.module), {}));
  const dry = useDryRuns(state.module);
  const [byState, setByState] = useState("");
  const [byFlow, setByFlow] = useState("");
  const [kind, setKind] = useState("");
  const [sort, setSort] = useState<RunSort>("start");
  const live = runs.some((r) => phaseOf(r).state !== "finished" && phaseOf(r).state !== "cancelled");
  const now = useNow(live);

  const rows = useMemo<Row[]>(() => {
    const all: Row[] = [
      ...runs.map((r) => ({
        id: r.id,
        flow: r.flow,
        title: r.title || r.subject || r.id,
        phase: phaseOf(r),
        created_at: Number(r.created_at ?? 0),
        elapsed: runElapsed(r, now),
        tokens: tokensOf(r.usage),
        dry: !!r.dry || isDryRun(r.id),
      })),
      ...dry.map((d) => ({
        id: d.id,
        flow: d.flow,
        title: d.id,
        phase: phaseOf(d),
        created_at: Number(d.created_at ?? 0),
        elapsed: runElapsed(d, now),
        tokens: 0,
        dry: true,
      })),
    ];
    const by: Record<RunSort, (a: Row, b: Row) => number> = {
      start: (a, b) => b.created_at - a.created_at,
      duration: (a, b) => (b.elapsed ?? 0) - (a.elapsed ?? 0),
      tokens: (a, b) => b.tokens - a.tokens,
    };
    return all
      .filter((r) => !byState || r.phase.state === byState)
      .filter((r) => !byFlow || r.flow === byFlow)
      .filter((r) => !kind || (kind === "dry") === r.dry)
      .sort((a, b) => by[sort](a, b) || b.id.localeCompare(a.id));
  }, [runs, dry, byState, byFlow, kind, sort, now]);
  const flows = [...new Set([...defs.list.map((d) => d.name), ...runs.map((r) => r.flow), ...dry.map((d) => d.flow)])];

  return (
    <div className="hx-flow-panel" data-testid="flow-runs">
      <section className="hx-flow-section">
        <div className="hx-flow-toolbar" data-testid="flow-runs-toolbar">
          <span className="hx-flow-grow" />
          <select aria-label={t("flowui.filter.state")} data-testid="flow-filter-state" value={byState} onChange={(e) => setByState(e.target.value)}>
            <option value="">{t("flowui.filter.allStates")}</option>
            {PHASE_STATES.map((s) => (
              <option key={s} value={s}>
                {t("flowui.phase." + s)}
              </option>
            ))}
          </select>
          <select aria-label={t("flowui.list.filterFlow")} data-testid="flow-filter-flow" value={byFlow} onChange={(e) => setByFlow(e.target.value)}>
            <option value="">{t("flowui.list.allFlows")}</option>
            {flows.map((name) => (
              <option key={name} value={name}>
                {titleOf(defs.list, name)}
              </option>
            ))}
          </select>
          <select aria-label={t("flowui.filter.kind")} data-testid="flow-filter-kind" value={kind} onChange={(e) => setKind(e.target.value)}>
            <option value="">{t("flowui.filter.allKinds")}</option>
            <option value="real">{t("flowui.filter.real")}</option>
            <option value="dry">{t("flowui.dry.badge")}</option>
          </select>
          <select aria-label={t("flowui.sort")} data-testid="flow-sort-runs" value={sort} onChange={(e) => setSort(e.target.value as RunSort)}>
            <option value="start">{t("flowui.sort.start")}</option>
            <option value="duration">{t("flowui.sort.duration")}</option>
            <option value="tokens">{t("flowui.sort.tokens")}</option>
          </select>
        </div>
        {rows.length === 0 ? <p className="hx-flow-hint">{t(runs.length || dry.length ? "flowui.list.noMatch" : "flowui.list.noRuns")}</p> : null}
        <ul className="hx-flow-rows" data-testid="flow-run-list">
          {rows.map((r) => (
            <li key={r.id}>
              <button
                type="button"
                className={`hx-flow-row${sel.run === r.id ? " on" : ""}`}
                data-testid={`flow-run-${r.id}`}
                data-phase={r.phase.state}
                data-dry={r.dry ? "true" : undefined}
                aria-current={sel.run === r.id ? "true" : undefined}
                onClick={() => (onPick ? onPick(r.id, r.flow) : state.showRun(sel.run === r.id ? null : r.id))}
              >
                <span className="hx-flow-row-title">
                  <PhaseBadge phase={r.phase} />
                  {r.title}
                  {r.dry ? <span className="hx-flow-badge">{t("flowui.dry.badge")}</span> : null}
                </span>
                <span className="hx-flow-faint">
                  {[titleOf(defs.list, r.flow), formatTime(r.created_at), formatDuration(r.elapsed), r.tokens ? t("flowui.tokens", { n: formatTokens(r.tokens) }) : ""].filter(Boolean).join(" · ")}
                </span>
              </button>
            </li>
          ))}
        </ul>
      </section>
      {sel.run ? null : (
        <p className="hx-flow-hint" data-testid="flow-run-pick-hint">
          {t("flowui.runs.pick")}
        </p>
      )}
    </div>
  );
}

/** A run's state as a pill: a spinner while it is pausing; why it waits, when it does. */
export function PhaseBadge({ phase, testid }: { phase: Phase; testid?: string }) {
  const t = useT();
  return (
    <span className={`hx-flow-pill hx-flow-phase hx-flow-tone-${phaseTone(phase.state)}`} data-testid={testid} data-phase={phase.state} data-reason={phase.reason ?? undefined}>
      {phase.state === "pausing" || phase.state === "running" ? <span className={`hx-flow-spin${phase.state === "running" ? " hx-flow-spin-slow" : ""}`} aria-hidden="true" /> : null}
      {t("flowui.phase." + phase.state)}
      {phase.state === "waiting" && phase.reason ? ` · ${t("flowui.reason." + phase.reason)}` : ""}
    </span>
  );
}

export function RunDetail({ state, run, station }: { state: FlowSpaceState; run: string; station: string | null }) {
  const t = useT();
  const view = useRunView(state.module, run);
  // A run just started has its main session a moment later: the middle goes to it then.
  const main = view?.record.session_keys?.[MAIN_SESSION_KEY] ?? null;
  useEffect(() => {
    if (main && state.get().run === run) void state.centre();
  }, [state, run, main]);
  if (!view) return <p className="hx-flow-hint">{t("flowui.loading")}</p>;
  const def = defOf(view);
  return (
    <section className="hx-flow-section" data-testid="flow-run-detail" data-run={view.record.id}>
      <RunHead view={view} module={state.module} />
      {isDryRun(run) ? <DryControls module={state.module} run={run} status={view.record.status} /> : null}
      {def ? <FlowGraph def={def} view={view} selected={station} onSelect={(s) => state.pick(station === s ? null : s)} /> : null}
      {def && station ? <StationPanel key={`${run}/${station}`} station={station} def={def} view={view} module={state.module} initialTab="run" /> : null}
      {isDryRun(run) ? null : <RunSessionsList module={state.module} run={run} rev={view.record.rev} />}
    </section>
  );
}

function RunHead({ view, module }: { view: RunView; module: string }) {
  const t = useT();
  const nest = useNest();
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

/** The run's sub-agents and tasks (`sessions {run}`): each opens in the middle. */
function RunSessionsList({ module, run, rev }: { module: string; run: string; rev: number }) {
  const t = useT();
  const nest = useNest();
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
  const locale = useLocale();
  useSelection(state);
  const lang = locale.startsWith("zh") ? "zh-CN" : "en";
  const guide = useMethod<Guide>(state.module, "guide", { lang });
  const box = useRef<HTMLDivElement | null>(null);
  return (
    <div className="hx-flow-panel hx-flow-reference" data-testid="flow-reference-tab" ref={box}>
      <FlowFramework onPick={(p) => box.current && revealInGuide(box.current, p)} />
      <GuideView state={state} guide={guide.value ?? null} error={guide.error} />
      <FlowReference module={state.module} />
    </div>
  );
}

/**
 * Bring what a picked element of the framework is about into view in the guide below: the row of
 * its term in its group (the last such row: a group's summary table comes before its objects), else
 * the group's heading; it is marked for a moment.
 */
export function revealInGuide(root: HTMLElement, pick: FrameworkPick): HTMLElement | null {
  const rows = [...root.querySelectorAll<HTMLElement>(`[data-guide-group="${pick.group}"] [data-guide-term]`)];
  const row = pick.term ? rows.filter((r) => r.dataset.guideTerm === pick.term).pop() : undefined;
  const target = row ?? root.querySelector<HTMLElement>(`[data-guide-heading="${pick.group}"]`);
  if (!target) return null;
  target.scrollIntoView?.({ behavior: "smooth", block: "start" });
  for (const el of root.querySelectorAll("[data-revealed]")) el.removeAttribute("data-revealed");
  target.setAttribute("data-revealed", "");
  setTimeout(() => target.removeAttribute("data-revealed"), 1600);
  return target;
}

/** The guide's markdown, drawn in its groups (each with its colour); each whole example can start a
 *  new flow in the general conversation. */
export function GuideView({ state, guide, error }: { state: FlowSpaceState; guide: Guide | null; error?: string | null }) {
  const t = useT();
  const nest = useNest();
  const sections = useMemo(() => (guide ? guideSections(guideBlocks(guide.text)) : []), [guide]);
  let example = 0;
  const part = (b: GuideBlock, i: number, group: GuideGroup | null, overview: boolean) => {
    if (b.kind === "code" && b.example) {
      const n = example++;
      return (
        <div key={i} className="hx-flow-example" data-testid={`flow-guide-example-${n}`}>
          <pre className="hx-flow-code hx-flow-mono">{b.text}</pre>
          <div className="hx-flow-actions">
            <span className="hx-flow-grow" />
            <button
              type="button"
              className="btn sm outline"
              data-testid={`flow-guide-like-${n}`}
              onClick={() => void startInGeneral(state, nest, t("flowui.guide.likePrompt", { yaml: b.text }))}
            >
              {t("flowui.guide.like")}
            </button>
          </div>
        </div>
      );
    }
    if (b.kind === "heading" && b.level === 2 && group !== null) {
      return (
        <h3 key={i} className={`hx-flow-guide-h hx-flow-guide-group-h hx-fw-g${group}`} data-guide-heading={group} data-testid={`flow-guide-group-${group}`}>
          <span className="hx-flow-guide-swatch" aria-hidden="true" />
          <Inline text={b.text} />
        </h3>
      );
    }
    if (b.kind === "list" && overview && b.items.length === 5) {
      return (
        <ul key={i} className="hx-flow-list hx-flow-guide-overview">
          {b.items.map((item, j) => (
            <li key={j} className={`hx-fw-g${j + 1}`} data-group={j + 1}>
              <span className="hx-flow-guide-swatch" aria-hidden="true" />
              <Inline text={item} />
            </li>
          ))}
        </ul>
      );
    }
    return <GuidePart key={i} block={b} />;
  };
  return (
    <section className="hx-flow-section hx-flow-guide" data-testid="flow-guide" data-lang={guide?.lang}>
      {error ? <div className="hx-flow-error">{error}</div> : null}
      {!guide && !error ? <p className="hx-flow-hint">{t("flowui.loading")}</p> : null}
      {sections.map((sec, k) => (
        <div
          key={k}
          className={sec.group ? `hx-flow-guide-sec hx-fw-g${sec.group}` : "hx-flow-guide-sec"}
          data-guide-group={sec.group ?? undefined}
          data-testid={sec.group ? `flow-guide-sec-${sec.group}` : undefined}
        >
          {sec.blocks.map((b, i) => part(b, i, sec.group, !!sec.overview))}
        </div>
      ))}
    </section>
  );
}

function Inline({ text }: { text: string }) {
  return (
    <>
      {inlineOf(text).map((p, i) => (p.kind === "code" ? <code key={i}>{p.text}</code> : p.kind === "strong" ? <strong key={i}>{p.text}</strong> : <span key={i}>{p.text}</span>))}
    </>
  );
}

function GuidePart({ block: b }: { block: GuideBlock }) {
  switch (b.kind) {
    case "heading": {
      const H = (`h${Math.min(6, b.level + 2)}` as unknown) as "h3";
      return (
        <H className="hx-flow-guide-h">
          <Inline text={b.text} />
        </H>
      );
    }
    case "paragraph":
      return (
        <p className="hx-flow-guide-p">
          <Inline text={b.text} />
        </p>
      );
    case "list":
      return (
        <ul className="hx-flow-list">
          {b.items.map((item, i) => (
            <li key={i}>
              <Inline text={item} />
            </li>
          ))}
        </ul>
      );
    case "table":
      return (
        <table className="hx-flow-table">
          {b.head.length ? (
            <thead>
              <tr>
                {b.head.map((h, i) => (
                  <th key={i}>
                    <Inline text={h} />
                  </th>
                ))}
              </tr>
            </thead>
          ) : null}
          <tbody>
            {b.rows.map((r, i) => (
              <tr key={i} data-guide-term={guideTermOf(r[0]) || undefined}>
                {r.map((c, j) => (
                  <td key={j}>
                    <Inline text={c} />
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      );
    case "code":
      return <pre className="hx-flow-code hx-flow-mono">{b.text}</pre>;
  }
}
