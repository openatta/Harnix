/**
 * Schedules as rows — the same in a session's list, a project's block and the console: the title,
 * when (in words, the zone said when it is not the reader's), what it does, its state and source,
 * when it is next due (the reader's time, and how far off), how the last delivery went; run now,
 * pause or resume, edit, delete, and its deliveries opened beneath it, each leading to the session
 * or task it went to. The models' jobs are rows too: read-only, deletable.
 */

import { useEffect, useState } from "react";
import type { History, ModelJob, Receipt, Schedule } from "@harnix/core";
import { Glyph, t, useT, type HostApi, type Registry } from "@harnix/react";

import { errorText, openReceipt, openSessionAt, reasonText, sessionName, targetText } from "./common";
import { scheduleEditor } from "./editor";
import { describeTiming } from "./frequency";
import { stateOf } from "./store";
import { formatWhen, localZone, whenText } from "./time";

/** How many deliveries a history reads at a time. */
const PAGE = 20;

/** Run a schedule now, and say how it went. */
export async function runNow(host: HostApi, s: Schedule): Promise<Receipt | null> {
  try {
    const receipt = await host.call<Receipt>("schedule.runNow", { id: s.id });
    const why = reasonText(receipt.reason);
    if (receipt.outcome === "failed") host.shell.notice(t("schedule.ran.failed", { title: s.title, why }), "error");
    else host.shell.notice(t(`schedule.ran.${receipt.outcome}`, { title: s.title, why }));
    return receipt;
  } catch (e) {
    host.shell.notice(errorText(e), "error");
    return null;
  }
}

/** A call whose failure is the banner's. */
async function attempt(host: HostApi, fn: () => Promise<unknown>): Promise<void> {
  try {
    await fn();
  } catch (e) {
    host.shell.notice(errorText(e), "error");
  }
}

export function ScheduleRows({ host, r, schedules, model = [], empty }: { host: HostApi; r: Registry; schedules: Schedule[]; model?: ModelJob[]; empty?: string }) {
  const tt = useT();
  if (!schedules.length && !model.length) return <div className="hx-sched-empty">{empty ?? tt("schedule.none")}</div>;
  return (
    <div className="hx-sched-rows" data-testid="schedule-rows">
      {schedules.map((s) => (
        <ScheduleRow key={s.id} host={host} r={r} schedule={s} />
      ))}
      {model.map((job) => (
        <ModelRow key={`${job.instance}/${job.id}`} host={host} r={r} job={job} />
      ))}
    </div>
  );
}

export function ScheduleRow({ host, r, schedule: s }: { host: HostApi; r: Registry; schedule: Schedule }) {
  const tt = useT();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const state = stateOf(s);
  const zone = s.time_zone !== localZone() ? s.time_zone : null;
  const last = s.last_delivery;
  const act = (fn: () => Promise<unknown>) => async () => {
    setBusy(true);
    try {
      await fn();
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="hx-sched-row" data-testid="schedule-row" data-schedule={s.id} data-state={state} data-source="user" data-kind={s.target.kind}>
      <div className="hx-sched-head">
        <span className={`hx-sched-dot is-${state}`} aria-hidden="true" />
        <span className="hx-sched-title" title={s.title}>
          {s.title}
        </span>
        <span className="hx-sched-chip" data-testid="schedule-state">
          {tt(`schedule.state.${state}`)}
        </span>
        <span className="hx-sched-chip quiet" data-testid="schedule-source" title={s.created_by}>
          {tt("schedule.source.user")}
        </span>
      </div>
      <div className="hx-sched-line" data-testid="schedule-when">
        {describeTiming(s.timing, s.time_zone)}
        {zone ? <span className="hx-sched-faint"> · {tt("schedule.zoneNote", { zone })}</span> : null}
      </div>
      <div className="hx-sched-line hx-sched-faint">{targetText(host, s.target)}</div>
      <div className="hx-sched-line" data-testid="schedule-next" data-next={s.next_fire ?? undefined}>
        {s.next_fire != null ? tt("schedule.next", { when: whenText(s.next_fire) }) : tt("schedule.next.none")}
      </div>
      {last ? (
        <div className="hx-sched-line hx-sched-faint" data-testid="schedule-last" data-outcome={last.outcome}>
          {tt("schedule.last", { outcome: tt(`schedule.outcome.${last.outcome}`), when: formatWhen(last.delivered_at ?? last.scheduled_at) })}
          {last.reason ? ` · ${reasonText(last.reason)}` : ""}
        </div>
      ) : null}
      <div className="hx-sched-actions">
        <button type="button" className="btn sm outline" data-testid="schedule-run-now" disabled={busy} onClick={act(() => runNow(host, s))}>
          {tt("schedule.runNow")}
        </button>
        <button
          type="button"
          className="btn sm ghost"
          data-testid="schedule-toggle"
          data-enabled={s.enabled ? "true" : "false"}
          disabled={busy}
          onClick={act(() => attempt(host, () => host.call(s.enabled ? "schedule.disable" : "schedule.enable", { id: s.id })))}
        >
          {tt(s.enabled ? "schedule.pause" : "schedule.resume")}
        </button>
        <button type="button" className="btn sm ghost" data-testid="schedule-edit" disabled={busy} onClick={() => void scheduleEditor(host, r, { existing: s, kinds: [s.target.kind] })}>
          {tt("schedule.edit")}
        </button>
        <button type="button" className="btn sm ghost" data-testid="schedule-history-toggle" aria-expanded={open} onClick={() => setOpen(!open)}>
          {tt(open ? "schedule.history.hide" : "schedule.history")}
        </button>
        <button
          type="button"
          className="btn sm ghost danger"
          data-testid="schedule-delete"
          disabled={busy}
          onClick={act(async () => {
            if (!(await host.dialogs.confirm(t("schedule.deleteConfirm", { title: s.title }), { danger: true, confirm: t("schedule.delete") }))) return;
            await attempt(host, () => host.call("schedule.delete", { id: s.id }));
          })}
        >
          {tt("schedule.delete")}
        </button>
      </div>
      {open ? <Deliveries host={host} r={r} schedule={s} /> : null}
    </div>
  );
}

/** A schedule's deliveries, newest first, a page at a time; read again when it was delivered once more. */
export function Deliveries({ host, r, schedule: s }: { host: HostApi; r: Registry; schedule: Schedule }) {
  const tt = useT();
  const [limit, setLimit] = useState(PAGE);
  const [history, setHistory] = useState<History | null>(null);
  const [error, setError] = useState<string | null>(null);
  const latest = s.last_delivery ? `${s.last_delivery.id}:${s.last_delivery.outcome}` : "";
  useEffect(() => {
    let live = true;
    host
      .call<History>("schedule.history", { id: s.id, limit })
      .then((h) => {
        if (!live) return;
        setHistory(h);
        setError(null);
      })
      .catch((e: unknown) => live && setError(errorText(e)));
    return () => {
      live = false;
    };
  }, [host, s.id, limit, latest]);
  if (error) return <div className="hx-sched-error">{error}</div>;
  if (!history) return <div className="hx-sched-empty">{tt("schedule.loading")}</div>;
  return (
    <div className="hx-sched-history" data-testid="schedule-history" data-total={history.total}>
      <div className="hx-sched-faint">{tt("schedule.receipts", { count: history.total })}</div>
      {history.receipts.length ? null : <div className="hx-sched-empty">{tt("schedule.receipts.none")}</div>}
      {history.receipts.map((receipt) => (
        <div key={receipt.id} className="hx-sched-receipt" data-testid="schedule-receipt" data-receipt={receipt.id} data-outcome={receipt.outcome}>
          <span className="hx-sched-when">{formatWhen(receipt.scheduled_at, s.time_zone)}</span>
          <span className={`hx-sched-chip is-${receipt.outcome}`}>{tt(`schedule.outcome.${receipt.outcome}`)}</span>
          {receipt.manual ? <span className="hx-sched-faint">{tt("schedule.receipt.manual")}</span> : null}
          {receipt.target?.created ? <span className="hx-sched-faint">{tt("schedule.receipt.created")}</span> : null}
          {receipt.reason ? <span className="hx-sched-faint hx-sched-reason">{reasonText(receipt.reason)}</span> : null}
          {receipt.target?.run || receipt.target?.session_id ? (
            <button type="button" className="btn sm ghost" data-testid="schedule-receipt-open" onClick={() => void openReceipt(host, r, s, receipt)}>
              {tt(receipt.target.run ? "schedule.receipt.openTask" : "schedule.receipt.openSession")}
            </button>
          ) : null}
        </div>
      ))}
      {history.total > history.receipts.length ? (
        <button type="button" className="btn sm ghost" data-testid="schedule-receipts-more" onClick={() => setLimit(limit + PAGE)}>
          {tt("schedule.receipts.more")}
        </button>
      ) : null}
      <div className="hx-sched-faint">{tt("schedule.receipts.note")}</div>
    </div>
  );
}

/** A job a model made in an engine: when (five fields, UTC), what it says, in which session; delete only. */
export function ModelRow({ host, r, job }: { host: HostApi; r: Registry; job: ModelJob }) {
  const tt = useT();
  const [busy, setBusy] = useState(false);
  const name = sessionName(host, job.instance, job.session_id);
  return (
    <div className="hx-sched-row" data-testid="schedule-model-row" data-job={job.id} data-instance={job.instance} data-source="model" title={tt("schedule.model.readOnly")}>
      <div className="hx-sched-head">
        <span className="hx-sched-dot is-on" aria-hidden="true" />
        <span className="hx-sched-title" title={job.prompt}>
          {job.prompt}
        </span>
        <span className="hx-sched-chip quiet" data-testid="schedule-source">
          {tt("schedule.source.model")}
        </span>
      </div>
      <div className="hx-sched-line">
        {tt("schedule.model.cron", { cron: job.cron })}
        {job.recurring ? "" : ` · ${tt("schedule.model.once")}`}
      </div>
      <div className="hx-sched-line">{job.next_fire != null ? tt("schedule.next", { when: whenText(job.next_fire) }) : tt("schedule.next.none")}</div>
      <div className="hx-sched-actions">
        <button type="button" className="btn sm ghost" data-testid="schedule-model-open" onClick={() => void openSessionAt(host, r, { instance: job.instance, session_id: job.session_id }).catch(() => undefined)}>
          <Glyph name="chat" className="glyph" />
          {tt("schedule.model.session", { name })}
        </button>
        <button
          type="button"
          className="btn sm ghost danger"
          data-testid="schedule-model-delete"
          disabled={busy}
          onClick={async () => {
            if (!(await host.dialogs.confirm(t("schedule.model.deleteConfirm"), { danger: true, confirm: t("schedule.delete") }))) return;
            setBusy(true);
            await attempt(host, () => host.call("schedule.model.delete", { instance: job.instance, id: job.id }));
            setBusy(false);
          }}
        >
          {tt("schedule.delete")}
        </button>
      </div>
    </div>
  );
}
