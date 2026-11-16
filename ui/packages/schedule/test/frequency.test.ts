/**
 * The guided frequency and the timing it is sent as, both ways: workdays as a week of Monday to
 * Friday, a month's day as a cron, a timing read back as the most specific frequency that says it;
 * what is wrong with one; and a timing in words.
 */

import { describe, expect, it } from "vitest";
import { Registry, setLocale } from "@harnix/react";

import { cronShape, defaultFrequency, describeTiming, fromTiming, frequencyProblems, monthlyOf, toTiming, type Frequency } from "../src/frequency";
import { momentOf } from "../src/time";
import { registerScheduleStrings } from "../src/strings";

setLocale("en");
registerScheduleStrings(new Registry());

const ZONE = "Asia/Shanghai";
const NOW = Date.parse("2026-11-16T00:00:00Z");
const base = defaultFrequency(NOW, ZONE);
const freq = (over: Partial<Frequency>): Frequency => ({ ...base, ...over });

describe("frequency → timing", () => {
  it("each kind as the scheduler takes it", () => {
    expect(toTiming(freq({ kind: "once", date: "2026-12-01", time: "08:30" }))).toEqual({ kind: "once", at: "2026-12-01T08:30" });
    expect(toTiming(freq({ kind: "after", after: 2, afterUnit: "hours" }))).toEqual({ kind: "after", seconds: 7200 });
    expect(toTiming(freq({ kind: "after", after: 3, afterUnit: "days" }))).toEqual({ kind: "after", seconds: 259200 });
    expect(toTiming(freq({ kind: "every", every: 15, everyUnit: "minutes" }))).toEqual({ kind: "every", minutes: 15 });
    expect(toTiming(freq({ kind: "every", every: 2, everyUnit: "hours" }))).toEqual({ kind: "every", minutes: 120 });
    expect(toTiming(freq({ kind: "daily", time: "09:00" }))).toEqual({ kind: "daily", time: "09:00" });
    expect(toTiming(freq({ kind: "weekly", weekdays: [5, 1, 3, 1], time: "07:15" }))).toEqual({ kind: "weekly", weekdays: [1, 3, 5], time: "07:15" });
    expect(toTiming(freq({ kind: "cron", expr: "  */5  9-17 * * 1-5 " }))).toEqual({ kind: "cron", expr: "*/5 9-17 * * 1-5" });
  });

  it("workdays are a week of Monday to Friday; a day of the month is a cron", () => {
    expect(toTiming(freq({ kind: "workdays", time: "09:00", weekdays: [6] }))).toEqual({ kind: "weekly", weekdays: [1, 2, 3, 4, 5], time: "09:00" });
    expect(toTiming(freq({ kind: "monthly", day: 1, time: "09:00" }))).toEqual({ kind: "cron", expr: "0 9 1 * *" });
    expect(toTiming(freq({ kind: "monthly", day: 31, time: "23:45" }))).toEqual({ kind: "cron", expr: "45 23 31 * *" });
  });
});

describe("timing → frequency", () => {
  it("as the most specific frequency that says it", () => {
    expect(fromTiming({ kind: "weekly", weekdays: [1, 2, 3, 4, 5], time: "09:00" }, ZONE, base)).toMatchObject({ kind: "workdays", time: "09:00" });
    expect(fromTiming({ kind: "weekly", weekdays: [2, 4], time: "10:00" }, ZONE, base)).toMatchObject({ kind: "weekly", weekdays: [2, 4] });
    expect(fromTiming({ kind: "cron", expr: "0 9 1 * *" }, ZONE, base)).toMatchObject({ kind: "monthly", day: 1, time: "09:00" });
    expect(fromTiming({ kind: "cron", expr: "0 9 * * 1" }, ZONE, base)).toMatchObject({ kind: "cron", expr: "0 9 * * 1" });
    expect(fromTiming({ kind: "every", minutes: 180, from: 1 }, ZONE, base)).toMatchObject({ kind: "every", every: 3, everyUnit: "hours" });
    expect(fromTiming({ kind: "every", minutes: 45 }, ZONE, base)).toMatchObject({ kind: "every", every: 45, everyUnit: "minutes" });
    expect(fromTiming({ kind: "after", seconds: 5400 }, ZONE, base)).toMatchObject({ kind: "after", after: 90, afterUnit: "minutes" });
    expect(fromTiming({ kind: "after", seconds: 172800 }, ZONE, base)).toMatchObject({ kind: "after", after: 2, afterUnit: "days" });
    expect(fromTiming({ kind: "daily", time: "06:00" }, ZONE, base)).toMatchObject({ kind: "daily", time: "06:00" });
  });

  it("a single time: the wall clock as written, or one with an offset in the schedule's zone", () => {
    expect(fromTiming({ kind: "once", at: "2026-12-01T08:30" }, ZONE, base)).toMatchObject({ kind: "once", date: "2026-12-01", time: "08:30" });
    expect(fromTiming({ kind: "once", at: "2026-12-01T08:30:00" }, ZONE, base)).toMatchObject({ date: "2026-12-01", time: "08:30" });
    expect(fromTiming({ kind: "once", at: "2026-12-01T00:30:00Z" }, ZONE, base)).toMatchObject({ date: "2026-12-01", time: "08:30" });
  });

  it("round trips, and a switch of kind keeps the others' values", () => {
    for (const f of [freq({ kind: "workdays", time: "08:00" }), freq({ kind: "monthly", day: 15, time: "12:05" }), freq({ kind: "weekly", weekdays: [6, 7], time: "10:00" })]) {
      expect(toTiming(fromTiming(toTiming(f), ZONE, base))).toEqual(toTiming(f));
    }
    const back = fromTiming({ kind: "daily", time: "06:00" }, ZONE, freq({ expr: "1 2 3 4 5", day: 9 }));
    expect([back.expr, back.day]).toEqual(["1 2 3 4 5", 9]);
    expect(monthlyOf("0 24 1 * *")).toBeNull();
    expect(monthlyOf("0 9 0 * *")).toBeNull();
  });
});

describe("what is wrong with one", () => {
  it("by the field it is about", () => {
    expect(frequencyProblems(freq({ kind: "daily", time: "09:00" }), ZONE, NOW)).toEqual({});
    expect(frequencyProblems(freq({ kind: "daily", time: "25:00" }), ZONE, NOW)).toEqual({ time: "schedule.err.time" });
    expect(frequencyProblems(freq({ kind: "weekly", weekdays: [] }), ZONE, NOW)).toEqual({ weekdays: "schedule.err.weekdays" });
    expect(frequencyProblems(freq({ kind: "every", every: 0 }), ZONE, NOW)).toEqual({ every: "schedule.err.amount" });
    expect(frequencyProblems(freq({ kind: "every", every: 1.5 }), ZONE, NOW)).toEqual({ every: "schedule.err.amount" });
    expect(frequencyProblems(freq({ kind: "after", after: Number.NaN }), ZONE, NOW)).toEqual({ after: "schedule.err.amount" });
    expect(frequencyProblems(freq({ kind: "monthly", day: 32 }), ZONE, NOW)).toEqual({ day: "schedule.err.day" });
    expect(frequencyProblems(freq({ kind: "cron", expr: "0 9 * *" }), ZONE, NOW)).toEqual({ expr: "schedule.err.cron" });
    expect(frequencyProblems(freq({ kind: "cron", expr: "0 9 * * MON" }), ZONE, NOW)).toEqual({ expr: "schedule.err.cron" });
  });

  it("a single time already past, in the schedule's zone; a date that is not one", () => {
    // 08:30 in Shanghai on the 16th is 00:30 UTC: past at 01:00 UTC, still to come at 00:00 UTC.
    const once = freq({ kind: "once", date: "2026-11-16", time: "08:30" });
    expect(frequencyProblems(once, ZONE, Date.parse("2026-11-16T01:00:00Z"))).toEqual({ when: "schedule.err.past" });
    expect(frequencyProblems(once, ZONE, Date.parse("2026-11-16T00:00:00Z"))).toEqual({});
    expect(frequencyProblems(once, "UTC", Date.parse("2026-11-16T01:00:00Z"))).toEqual({});
    expect(frequencyProblems(freq({ kind: "once", date: "2026-02-30", time: "08:00" }), ZONE, NOW)).toEqual({ when: "schedule.err.when" });
  });

  it("a cron's shape: numbers, *, ranges, lists and steps; whether it is ever due is the scheduler's", () => {
    expect(cronShape("*/15 9-17 1,15 * 1-5")).toBe(true);
    expect(cronShape("0 0 31 2 *")).toBe(true);
    expect(cronShape("@daily")).toBe(false);
    expect(cronShape("0 9 * * * *")).toBe(false);
  });
});

describe("a timing in words", () => {
  it("each kind", () => {
    expect(describeTiming({ kind: "daily", time: "09:00" }, ZONE)).toBe("Daily at 09:00");
    expect(describeTiming({ kind: "weekly", weekdays: [1, 2, 3, 4, 5], time: "09:00" }, ZONE)).toBe("Workdays at 09:00");
    expect(describeTiming({ kind: "weekly", weekdays: [3, 1], time: "09:00" }, ZONE)).toBe("Mon, Wed at 09:00");
    expect(describeTiming({ kind: "cron", expr: "0 9 1 * *" }, ZONE)).toBe("Monthly on day 1 at 09:00");
    expect(describeTiming({ kind: "cron", expr: "0 9 * * 1" }, ZONE)).toBe("cron 0 9 * * 1");
    expect(describeTiming({ kind: "every", minutes: 120 }, ZONE)).toBe("Every 2 h");
    expect(describeTiming({ kind: "every", minutes: 15 }, ZONE)).toBe("Every 15 min");
    expect(describeTiming({ kind: "after", seconds: 3600 }, ZONE)).toBe("Once, 1 h after it is made");
  });

  it("a single time in its own zone, whatever the reader's", () => {
    const text = describeTiming({ kind: "once", at: "2026-12-01T08:30" }, ZONE);
    expect(text).toMatch(/^Once, /);
    expect(text).toContain("08:30");
    const at = momentOf("2026-12-01", "08:30", ZONE)!;
    expect(describeTiming({ kind: "after", seconds: 60, from: at - 60_000 }, ZONE)).toBe(text);
  });
});
