/**
 * The editor's guided frequency, and the timing it is sent as (`ScheduleTiming`, harnix-app.md §9):
 *
 * | frequency | timing |
 * |---|---|
 * | once, at a date and time | `once {at: "YYYY-MM-DDTHH:MM"}`, the wall clock of the schedule's zone |
 * | once, after N minutes, hours or days | `after {seconds}` |
 * | every N minutes or hours | `every {minutes}` |
 * | daily | `daily {time}` |
 * | weekly, on weekday chips | `weekly {weekdays, time}` (Monday 1 … Sunday 7) |
 * | workdays | `weekly {weekdays: [1..5], time}` |
 * | monthly, on a day of the month | `cron {expr: "M H D * *"}` |
 * | custom | `cron {expr}`, five fields, as the scheduler reads them |
 *
 * And back, for an edit: a timing is read as the most specific frequency that says it — Monday to
 * Friday is "workdays", a cron of one minute, one hour and one day of every month is "monthly".
 * One frequency is one draft: switching between kinds keeps what the others were set to.
 *
 * A problem caught here is a key of this package's words; the scheduler has the last word (a
 * cron that is never due, a zone it does not know: `INVALID_PARAMS`, shown where it applies).
 */

import type { ScheduleTiming } from "@harnix/core";
import { t } from "@harnix/react";

import { formatWhen, isTime, localZone, momentOf, wallClockOf } from "./time";

export const FREQUENCY_KINDS = ["once", "after", "every", "daily", "weekly", "workdays", "monthly", "cron"] as const;
export type FrequencyKind = (typeof FREQUENCY_KINDS)[number];

export const WORKDAYS = [1, 2, 3, 4, 5];

/** The editor's frequency: every kind's fields, so a switch of kind loses nothing. */
export interface Frequency {
  kind: FrequencyKind;
  /** once: the date, `YYYY-MM-DD`. */
  date: string;
  /** once, daily, weekly, workdays, monthly: `HH:MM`. */
  time: string;
  /** after: how many `afterUnit`s. */
  after: number;
  afterUnit: "minutes" | "hours" | "days";
  /** every: how many `everyUnit`s. */
  every: number;
  everyUnit: "minutes" | "hours";
  /** weekly: Monday 1 … Sunday 7. */
  weekdays: number[];
  /** monthly: the day of the month, 1–31. */
  day: number;
  /** cron: five fields. */
  expr: string;
}

const UNIT_SECONDS = { minutes: 60, hours: 3600, days: 86400 } as const;

/** A new schedule's frequency: daily at nine, tomorrow for a single time. */
export function defaultFrequency(now = Date.now(), zone = localZone()): Frequency {
  const tomorrow = wallClockOf(now + 86_400_000, zone).date;
  return { kind: "daily", date: tomorrow, time: "09:00", after: 30, afterUnit: "minutes", every: 30, everyUnit: "minutes", weekdays: [1], day: 1, expr: "0 9 * * *" };
}

/** `HH:MM` of an hour and a minute. */
const hhmm = (h: number, m: number) => `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}`;

/** The timing a frequency is sent as. */
export function toTiming(f: Frequency): ScheduleTiming {
  switch (f.kind) {
    case "once":
      return { kind: "once", at: `${f.date}T${f.time}` };
    case "after":
      return { kind: "after", seconds: Math.round(f.after * UNIT_SECONDS[f.afterUnit]) };
    case "every":
      return { kind: "every", minutes: Math.round(f.every * (f.everyUnit === "hours" ? 60 : 1)) };
    case "daily":
      return { kind: "daily", time: f.time };
    case "weekly":
      return { kind: "weekly", weekdays: [...new Set(f.weekdays)].sort((a, b) => a - b), time: f.time };
    case "workdays":
      return { kind: "weekly", weekdays: [...WORKDAYS], time: f.time };
    case "monthly": {
      const [h, m] = f.time.split(":").map(Number);
      return { kind: "cron", expr: `${m ?? 0} ${h ?? 0} ${f.day} * *` };
    }
    case "cron":
      return { kind: "cron", expr: f.expr.trim().replace(/\s+/g, " ") };
  }
}

/** A cron of one minute, one hour and one day of every month: that day's time. */
export function monthlyOf(expr: string): { day: number; time: string } | null {
  const m = /^(\d{1,2}) (\d{1,2}) (\d{1,2}) \* \*$/.exec(expr.trim().replace(/\s+/g, " "));
  if (!m) return null;
  const [mi, h, d] = [Number(m[1]), Number(m[2]), Number(m[3])];
  if (mi > 59 || h > 23 || d < 1 || d > 31) return null;
  return { day: d, time: hhmm(h, mi) };
}

/**
 * A timing read back as the frequency that says it, over `base` (what the other kinds keep). A
 * `once` with an offset is shown as the wall clock of the schedule's zone.
 */
export function fromTiming(timing: ScheduleTiming, zone: string, base: Frequency = defaultFrequency(Date.now(), zone)): Frequency {
  const f = { ...base, weekdays: [...base.weekdays] };
  switch (timing.kind) {
    case "once": {
      const local = /^(\d{4}-\d{2}-\d{2})T(\d{2}:\d{2})(:\d{2}(\.\d+)?)?$/.exec(timing.at);
      if (local) return { ...f, kind: "once", date: local[1]!, time: local[2]! };
      const ms = Date.parse(timing.at);
      if (Number.isNaN(ms)) return { ...f, kind: "once" };
      const w = wallClockOf(ms, zone);
      return { ...f, kind: "once", date: w.date, time: w.time };
    }
    case "after": {
      const s = timing.seconds;
      const unit = s % 86400 === 0 ? "days" : s % 3600 === 0 ? "hours" : "minutes";
      return { ...f, kind: "after", afterUnit: unit, after: Math.max(1, Math.round(s / UNIT_SECONDS[unit])) };
    }
    case "every":
      return timing.minutes % 60 === 0 ? { ...f, kind: "every", everyUnit: "hours", every: timing.minutes / 60 } : { ...f, kind: "every", everyUnit: "minutes", every: timing.minutes };
    case "daily":
      return { ...f, kind: "daily", time: timing.time };
    case "weekly": {
      const days = [...new Set(timing.weekdays)].sort((a, b) => a - b);
      if (days.join() === WORKDAYS.join()) return { ...f, kind: "workdays", time: timing.time, weekdays: days };
      return { ...f, kind: "weekly", time: timing.time, weekdays: days };
    }
    case "cron": {
      const monthly = monthlyOf(timing.expr);
      if (monthly) return { ...f, kind: "monthly", day: monthly.day, time: monthly.time, expr: timing.expr };
      return { ...f, kind: "cron", expr: timing.expr };
    }
  }
}

/** One cron field: numbers, `*`, ranges, lists and steps — names and macros are not read. */
const CRON_FIELD = /^(\*|\d+(-\d+)?)(\/\d+)?(,(\*|\d+(-\d+)?)(\/\d+)?)*$/;

/** Whether an expression has the shape of five cron fields; the scheduler says whether it is ever due. */
export function cronShape(expr: string): boolean {
  const fields = expr.trim().split(/\s+/);
  return fields.length === 5 && fields.every((x) => CRON_FIELD.test(x));
}

/**
 * What is wrong with a frequency, by the field it is about, as a key of this package's words; none,
 * an empty object. A single time already past counts (in the schedule's zone, at `now`).
 */
export function frequencyProblems(f: Frequency, zone: string, now = Date.now()): Record<string, string> {
  const out: Record<string, string> = {};
  const positive = (n: number) => Number.isInteger(n) && n >= 1;
  switch (f.kind) {
    case "once": {
      const at = momentOf(f.date, f.time, zone);
      if (at === null) out.when = "schedule.err.when";
      else if (at <= now) out.when = "schedule.err.past";
      break;
    }
    case "after":
      if (!positive(f.after)) out.after = "schedule.err.amount";
      break;
    case "every":
      if (!positive(f.every)) out.every = "schedule.err.amount";
      break;
    case "weekly":
      if (!f.weekdays.length) out.weekdays = "schedule.err.weekdays";
      if (!isTime(f.time)) out.time = "schedule.err.time";
      break;
    case "monthly":
      if (!Number.isInteger(f.day) || f.day < 1 || f.day > 31) out.day = "schedule.err.day";
      if (!isTime(f.time)) out.time = "schedule.err.time";
      break;
    case "daily":
    case "workdays":
      if (!isTime(f.time)) out.time = "schedule.err.time";
      break;
    case "cron":
      if (!cronShape(f.expr)) out.expr = "schedule.err.cron";
      break;
  }
  return out;
}

/** Weekdays as the page writes them: "周一、周三", "Mon, Wed"; Monday to Friday is "工作日". */
export function weekdaysText(days: number[]): string {
  const sorted = [...new Set(days)].sort((a, b) => a - b);
  return sorted.map((d) => t(`schedule.wd.${d}`)).join(t("schedule.wd.sep"));
}

/**
 * A timing in words, in its zone: "每天 09:00", "工作日 09:00", "每月 1 日 09:00", "每 15 分钟".
 * `after` with the moment it counts from is written as the moment it falls on.
 */
export function describeTiming(timing: ScheduleTiming, zone: string): string {
  switch (timing.kind) {
    case "once": {
      const f = fromTiming(timing, zone);
      const at = momentOf(f.date, f.time, zone);
      return t("schedule.freq.onceAt", { when: at === null ? timing.at : formatWhen(at, zone) });
    }
    case "after": {
      if (timing.from != null) return t("schedule.freq.onceAt", { when: formatWhen(timing.from + timing.seconds * 1000, zone) });
      const f = fromTiming(timing, zone);
      return t("schedule.freq.after", { amount: t(`schedule.unit.${f.afterUnit}`, { count: f.after }) });
    }
    case "every":
      return timing.minutes % 60 === 0 ? t("schedule.freq.everyHours", { count: timing.minutes / 60 }) : t("schedule.freq.everyMinutes", { count: timing.minutes });
    case "daily":
      return t("schedule.freq.daily", { time: timing.time });
    case "weekly": {
      const days = [...new Set(timing.weekdays)].sort((a, b) => a - b);
      if (days.join() === WORKDAYS.join()) return t("schedule.freq.workdays", { time: timing.time });
      if (days.length === 7) return t("schedule.freq.daily", { time: timing.time });
      return t("schedule.freq.weekly", { days: weekdaysText(days), time: timing.time });
    }
    case "cron": {
      const monthly = monthlyOf(timing.expr);
      if (monthly) return t("schedule.freq.monthly", { day: monthly.day, time: monthly.time });
      return t("schedule.freq.cron", { expr: timing.expr });
    }
  }
}
