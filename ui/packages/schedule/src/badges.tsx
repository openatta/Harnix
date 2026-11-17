/**
 * The clocks: on a session's row in the left column, while a schedule that is on turns to it (or a
 * model's job is in it); and on a task's row, the next time of the schedule that started it while
 * that one is on. Both read the store's last value, so a row costs no call.
 */

import { useSyncExternalStore } from "react";
import { Glyph, useT } from "@harnix/react";

import { activeOfSession, scheduleOfRun, stateOf, type ScheduleStore } from "./store";
import { relativeTo, whenText } from "./time";

/** How many titles a clock's tooltip names before "and N more". */
const NAMED = 2;

export function useSchedules(store: ScheduleStore) {
  return useSyncExternalStore(store.subscribe, store.getSnapshot);
}

export function SessionClock({ store, instance, sessionId }: { store: ScheduleStore; instance: string; sessionId: string }) {
  const t = useT();
  const list = useSchedules(store);
  if (!list) return null;
  const active = activeOfSession(list, instance, sessionId);
  const count = active.schedules.length + active.model.length;
  if (!count) return null;
  const titles = [...active.schedules.map((s) => (s.next_fire != null ? `${s.title} · ${whenText(s.next_fire)}` : s.title)), ...active.model.map((j) => j.prompt)];
  const named = titles.slice(0, NAMED).join("; ");
  const more = titles.length > NAMED ? ` ${t("schedule.badge.more", { count: titles.length - NAMED })}` : "";
  const label = t("schedule.badge.session", { titles: named }) + more;
  return (
    <span className="hx-sched-clock" data-testid="session-schedule-badge" data-count={count} title={label} aria-label={label} role="img">
      <Glyph name="schedule.clock" className="glyph hx-sched-glyph" />
    </span>
  );
}

export function TaskNext({ store, run }: { store: ScheduleStore; run: { trigger?: { kind: string; schedule?: string } | null } }) {
  const t = useT();
  const list = useSchedules(store);
  const schedule = list ? scheduleOfRun(list, run) : null;
  if (!schedule || stateOf(schedule) !== "on" || schedule.next_fire == null) return null;
  return (
    <span className="hx-sched-next" data-testid="task-schedule-next" data-schedule={schedule.id} title={t("schedule.badge.task", { title: schedule.title, when: whenText(schedule.next_fire) })}>
      {t("schedule.badge.next", { when: relativeTo(schedule.next_fire) })}
    </span>
  );
}
