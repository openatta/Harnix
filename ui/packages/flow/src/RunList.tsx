/**
 * The flows and their runs: the definitions this application has, a
 * form to start a run of one, and the runs from the `app.<module>.runs` topic, filtered by flow,
 * status and subject.
 */

import { useEffect, useMemo, useState } from "react";
import { useNest, useT } from "@harnix/react";

import { errorText, useMethod, useTopic } from "./hooks";
import { formatTime, methodName, openOf, runIdOf, runTone, runsOf, runsTopic } from "./model";
import type { DefSummary, RunStatus } from "./types";

export interface RunListProps {
  module?: string;
  /** The run shown, highlighted in the list. */
  selected?: string | null;
  onSelect?(run: string): void;
  /** The flow shown, highlighted among the definitions. */
  flow?: string | null;
  /** Given: the definitions are listed, and choosing one calls it. */
  onSelectFlow?(name: string): void;
}

const STATUSES: RunStatus[] = ["active", "finished", "cancelled"];

export function RunList({ module = "flow", selected, onSelect, flow, onSelectFlow }: RunListProps) {
  const t = useT();
  const defs = useMethod<DefSummary[] | { defs?: DefSummary[] }>(module, "defs", {});
  const list: DefSummary[] = Array.isArray(defs.value) ? defs.value : (defs.value?.defs ?? []);
  const runs = runsOf(useTopic<unknown>(runsTopic(module), {}));
  const [byFlow, setByFlow] = useState("");
  const [byStatus, setByStatus] = useState("");
  const [text, setText] = useState("");
  const [starting, setStarting] = useState(false);

  const shown = useMemo(() => {
    const q = text.trim().toLowerCase();
    return runs
      .filter((r) => (!byFlow || r.flow === byFlow) && (!byStatus || r.status === byStatus))
      .filter((r) => !q || [r.id, r.subject, r.title ?? ""].some((x) => String(x).toLowerCase().includes(q)))
      .sort((a, b) => Number(b.created_at ?? 0) - Number(a.created_at ?? 0) || String(b.id).localeCompare(String(a.id)));
  }, [runs, byFlow, byStatus, text]);

  return (
    <div className="hx-flow-sidebar" data-testid="flow-run-list">
      {onSelectFlow ? (
        <section className="hx-flow-section" data-testid="flow-defs">
          <div className="hx-flow-section-head">{t("flowui.list.flows")}</div>
          {defs.error ? <div className="hx-flow-error">{defs.error}</div> : null}
          <ul className="hx-flow-rows">
            {list.map((d) => (
              <li key={d.name}>
                <button
                  type="button"
                  className={`hx-flow-row${flow === d.name && !selected ? " on" : ""}`}
                  data-testid={`flow-def-${d.name}`}
                  aria-current={flow === d.name && !selected ? "true" : undefined}
                  onClick={() => onSelectFlow(d.name)}
                >
                  <span className="hx-flow-row-title">{d.title || d.name}</span>
                  <span className="hx-flow-faint">{d.name}</span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      <section className="hx-flow-section">
        <div className="hx-flow-section-head">
          <span>{t("flowui.list.runs")}</span>
          <span className="hx-flow-grow" />
          <button type="button" className="btn sm outline" data-testid="flow-start-open" aria-expanded={starting} onClick={() => setStarting(!starting)}>
            {t("flowui.start.open")}
          </button>
        </div>
        {starting ? (
          <StartForm
            module={module}
            defs={list}
            initial={flow ?? list[0]?.name ?? ""}
            onStarted={(run) => {
              setStarting(false);
              if (run) onSelect?.(run);
            }}
          />
        ) : null}
        <div className="hx-flow-filters">
          <select aria-label={t("flowui.list.filterFlow")} data-testid="flow-filter-flow" value={byFlow} onChange={(e) => setByFlow(e.target.value)}>
            <option value="">{t("flowui.list.allFlows")}</option>
            {[...new Set([...list.map((d) => d.name), ...runs.map((r) => r.flow)])].map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>
          <select aria-label={t("flowui.list.filterStatus")} data-testid="flow-filter-status" value={byStatus} onChange={(e) => setByStatus(e.target.value)}>
            <option value="">{t("flowui.list.allStatuses")}</option>
            {STATUSES.map((s) => (
              <option key={s} value={s}>
                {t("flowui.runStatus." + s)}
              </option>
            ))}
          </select>
          <input
            className="hx-flow-input"
            type="search"
            aria-label={t("flowui.list.filterText")}
            placeholder={t("flowui.list.filterText")}
            data-testid="flow-filter-text"
            value={text}
            onChange={(e) => setText(e.target.value)}
          />
        </div>
        {shown.length === 0 ? <p className="hx-flow-hint">{t(runs.length ? "flowui.list.noMatch" : "flowui.list.noRuns")}</p> : null}
        <ul className="hx-flow-rows">
          {shown.map((r) => (
            <li key={r.id}>
              <button
                type="button"
                className={`hx-flow-row${selected === r.id ? " on" : ""}`}
                data-testid={`flow-run-${r.id}`}
                data-status={r.status}
                aria-current={selected === r.id ? "true" : undefined}
                onClick={() => onSelect?.(r.id)}
              >
                <span className="hx-flow-row-title">
                  <span className={`hx-flow-dot hx-flow-tone-${runTone(r.status)}`} aria-hidden="true" />
                  {r.title || r.subject || r.id}
                </span>
                <span className="hx-flow-faint">
                  {r.flow} · {t("flowui.runStatus." + r.status)}
                  {openOf(r).length ? ` · ${openOf(r).join(", ")}` : ""} · {formatTime(r.created_at)}
                </span>
              </button>
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}

/** Start a run: a flow, a subject, an optional title and inputs as JSON. */
export function StartForm({ module = "flow", defs, initial, onStarted }: { module?: string; defs: DefSummary[]; initial: string; onStarted?(run: string | null): void }) {
  const t = useT();
  const nest = useNest();
  const [flow, setFlow] = useState(initial);
  const [subject, setSubject] = useState("demo");
  const [title, setTitle] = useState("");
  const [inputs, setInputs] = useState("{}");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!flow && initial) setFlow(initial);
  }, [flow, initial]);

  const start = async () => {
    let parsed: unknown = undefined;
    if (inputs.trim()) {
      try {
        parsed = JSON.parse(inputs);
      } catch (e) {
        setError(t("flowui.start.badJson", { error: errorText(e) }));
        return;
      }
    }
    setBusy(true);
    setError(null);
    try {
      const answer = await nest.call(methodName(module, "start"), {
        flow,
        subject: subject.trim(),
        inputs: parsed && typeof parsed === "object" && Object.keys(parsed).length ? parsed : undefined,
        title: title.trim() || undefined,
      });
      onStarted?.(runIdOf(answer));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="hx-flow-form" data-testid="flow-start-form">
      <label className="hx-flow-label" htmlFor="hx-flow-start-flow">
        {t("flowui.start.flow")}
      </label>
      <select id="hx-flow-start-flow" data-testid="flow-start-flow" value={flow} onChange={(e) => setFlow(e.target.value)}>
        {defs.map((d) => (
          <option key={d.name} value={d.name}>
            {d.title ? `${d.title} (${d.name})` : d.name}
          </option>
        ))}
      </select>
      <label className="hx-flow-label" htmlFor="hx-flow-start-subject">
        {t("flowui.start.subject")}
      </label>
      <input id="hx-flow-start-subject" className="hx-flow-input" data-testid="flow-start-subject" value={subject} onChange={(e) => setSubject(e.target.value)} />
      <label className="hx-flow-label" htmlFor="hx-flow-start-title">
        {t("flowui.start.title")}
      </label>
      <input id="hx-flow-start-title" className="hx-flow-input" data-testid="flow-start-title" value={title} onChange={(e) => setTitle(e.target.value)} />
      <label className="hx-flow-label" htmlFor="hx-flow-start-inputs">
        {t("flowui.start.inputs")}
      </label>
      <textarea id="hx-flow-start-inputs" className="hx-flow-input hx-flow-mono" data-testid="flow-start-inputs" rows={3} value={inputs} onChange={(e) => setInputs(e.target.value)} />
      {error ? (
        <div className="hx-flow-error" data-testid="flow-start-error" role="alert">
          {error}
        </div>
      ) : null}
      <div className="hx-flow-actions">
        <button type="button" className="btn sm primary" data-testid="flow-start-submit" disabled={busy || !flow || !subject.trim()} onClick={() => void start()}>
          {t("flowui.start.submit")}
        </button>
      </div>
    </div>
  );
}
