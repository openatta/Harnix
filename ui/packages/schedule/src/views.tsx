/**
 * Where the schedules are listed: a session's, in a dialog from its menu; a project's, as a block of
 * its overview; every one, as a block of the home page — with the models' jobs, and the
 * instances whose jobs could not be read. Each offers a new one, made for where it was opened.
 */

import { useMemo, useState } from "react";
import { DialogBody, DialogFoot, t, useT, type HostApi, type Registry } from "@harnix/react";

import { useSchedules } from "./badges";
import { scheduleEditor, type EditorContext } from "./editor";
import { ScheduleRows } from "./list";
import { ofProject, ofSession, ordered, stateOf, type ScheduleState, type ScheduleStore } from "./store";

/** A session, as its menu hands it here: the space it belongs to, and its name. */
export interface SessionAt {
  instance: string;
  session_id: string;
  space: string;
  name?: string | null;
}

/** A session's schedules, in a dialog: those that turn to it and the models' jobs in it; a new one. */
export function sessionSchedulesDialog(host: HostApi, r: Registry, store: ScheduleStore, session: SessionAt): Promise<true | null> {
  return host.dialogs.open<true>((close) => <SessionSchedules host={host} r={r} store={store} session={session} close={close} />, {
    title: t("schedule.session.title", { name: session.name || t("schedule.session.untitled") }),
    wide: true,
  });
}

function SessionSchedules({ host, r, store, session, close }: { host: HostApi; r: Registry; store: ScheduleStore; session: SessionAt; close(result: true | null): void }) {
  const tt = useT();
  const list = useSchedules(store);
  const mine = list ? ofSession(list, session.instance, session.session_id) : null;
  const context: EditorContext = { kinds: ["session"], space: session.space, session: { instance: session.instance, session_id: session.session_id, name: session.name } };
  return (
    <>
      <DialogBody>
        <div className="hx-sched-pane" data-testid="schedule-session" data-sid={session.session_id}>
          {mine ? <ScheduleRows host={host} r={r} schedules={ordered(mine.schedules)} model={mine.model} /> : <div className="hx-sched-empty">{tt("schedule.loading")}</div>}
          <div className="hint">{tt("schedule.notRunning")}</div>
        </div>
      </DialogBody>
      <DialogFoot>
        <button type="button" className="btn outline" onClick={() => close(null)}>
          {tt("common.close")}
        </button>
        <button type="button" className="btn primary" data-testid="schedule-new" onClick={() => void scheduleEditor(host, r, context)}>
          {tt("schedule.new")}
        </button>
      </DialogFoot>
    </>
  );
}

/** What a project's overview hands a block. */
export interface PanelContext {
  root: string;
  space?: string;
}

/** A project's schedules on its overview, and a new one: a new session or a task in the project. */
export function ProjectSchedules({ host, r, store, context }: { host: HostApi; r: Registry; store: ScheduleStore; context: PanelContext }) {
  const tt = useT();
  const list = useSchedules(store);
  const mine = list ? ordered(ofProject(list, context.root)) : null;
  return (
    <div className="hx-sched-panel" data-testid="overview-schedules">
      {mine ? <ScheduleRows host={host} r={r} schedules={mine} /> : <div className="hx-sched-empty">{tt("schedule.loading")}</div>}
      <button
        type="button"
        className="btn sm ghost"
        data-testid="schedule-new"
        onClick={() => void scheduleEditor(host, r, { kinds: ["new_session", "task"], project: context.root, space: context.space, shownIn: context.space })}
      >
        {tt("schedule.new")}
      </button>
    </div>
  );
}

const STATES: ScheduleState[] = ["on", "paused", "ended"];

/** Every schedule of the process, filtered by state and text; the models' jobs; a new one. */
export function AllSchedules({ host, r, store }: { host: HostApi; r: Registry; store: ScheduleStore }) {
  const tt = useT();
  const list = useSchedules(store);
  const [state, setState] = useState<"" | ScheduleState>("");
  const [text, setText] = useState("");
  const shown = useMemo(() => {
    const q = text.trim().toLowerCase();
    const prompt = (s: { target: { kind: string } & Record<string, unknown> }) => (typeof s.target.prompt === "string" ? s.target.prompt : typeof s.target.subject === "string" ? s.target.subject : "");
    return ordered(list?.schedules ?? []).filter((s) => (!state || stateOf(s) === state) && (!q || s.title.toLowerCase().includes(q) || prompt(s).toLowerCase().includes(q)));
  }, [list, state, text]);
  if (!list) {
    return (
      <div className="hx-sched-all" data-testid="schedules">
        {tt("schedule.loading")}
      </div>
    );
  }
  const filtered = Boolean(state || text.trim());
  return (
    <div className="hx-sched-all" data-testid="schedules">
      <div className="hx-sched-toolbar">
        <button type="button" className="btn sm primary" data-testid="schedule-new" onClick={() => void scheduleEditor(host, r, { kinds: ["new_session", "task"] })}>
          {tt("schedule.new")}
        </button>
        <select aria-label={tt("schedule.filter.state")} data-testid="schedule-filter-state" value={state} onChange={(e) => setState(e.target.value as "" | ScheduleState)}>
          <option value="">{tt("schedule.filter.all")}</option>
          {STATES.map((s) => (
            <option key={s} value={s}>
              {tt(`schedule.state.${s}`)}
            </option>
          ))}
        </select>
        <input className="hx-sched-input" type="search" data-testid="schedule-filter-text" aria-label={tt("schedule.filter.text")} placeholder={tt("schedule.filter.text")} value={text} onChange={(e) => setText(e.target.value)} />
      </div>
      <ScheduleRows host={host} r={r} schedules={shown} empty={filtered ? tt("schedule.filter.none") : tt("schedule.none")} />
      {list.model.length ? (
        <section className="hx-sched-section" data-testid="schedule-model">
          <h3>{tt("schedule.model.title")}</h3>
          <div className="hint">{tt("schedule.model.readOnly")}</div>
          <ScheduleRows host={host} r={r} schedules={[]} model={list.model} />
        </section>
      ) : null}
      {list.unreachable.length ? (
        <div className="hx-sched-note is-warn" data-testid="schedule-unreachable">
          {tt("schedule.unreachable", { instances: list.unreachable.join(", ") })}
        </div>
      ) : null}
      <div className="hint">{tt("schedule.notRunning")}</div>
    </div>
  );
}
