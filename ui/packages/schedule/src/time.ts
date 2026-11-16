/**
 * Times in a zone, as the page can work them out with `Intl` alone: this machine's zone, every zone
 * the browser knows, a moment as the wall clock of a zone and back, a zone's offset, and how the
 * page writes a time to come — in the reader's own zone, with how far off it is.
 *
 * The scheduler is the authority on when a schedule is due (harnix-app.md §9); these are for showing
 * a time and for catching a mistake before it is sent (a one-time moment already past).
 */

import { currentLocale, t } from "@harnix/react";

/** A wall-clock date and time: `YYYY-MM-DD`, `HH:MM`. */
export interface WallClock {
  date: string;
  time: string;
}

const pad = (n: number) => String(n).padStart(2, "0");

/** This machine's IANA zone; UTC when the browser will not say. */
export function localZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
}

/** Whether the browser knows the zone. */
export function isZone(zone: string): boolean {
  if (!zone) return false;
  try {
    new Intl.DateTimeFormat("en-US", { timeZone: zone });
    return true;
  } catch {
    return false;
  }
}

let zones: string[] | null = null;

/** Every IANA zone the browser knows, UTC among them, sorted. */
export function allZones(): string[] {
  if (zones) return zones;
  let known: string[] = [];
  try {
    known = (Intl as unknown as { supportedValuesOf?: (key: string) => string[] }).supportedValuesOf?.("timeZone") ?? [];
  } catch {
    known = [];
  }
  zones = [...new Set(["UTC", localZone(), ...known])].sort();
  return zones;
}

const formatters = new Map<string, Intl.DateTimeFormat>();

/** The parts of a moment's wall clock in a zone. */
function partsIn(ms: number, zone: string): { year: number; month: number; day: number; hour: number; minute: number; second: number } {
  let f = formatters.get(zone);
  if (!f) {
    f = new Intl.DateTimeFormat("en-US", { timeZone: zone, hourCycle: "h23", year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit" });
    formatters.set(zone, f);
  }
  const out: Record<string, number> = {};
  for (const p of f.formatToParts(new Date(ms))) if (p.type !== "literal") out[p.type] = Number(p.value);
  return { year: out.year ?? 1970, month: out.month ?? 1, day: out.day ?? 1, hour: (out.hour ?? 0) % 24, minute: out.minute ?? 0, second: out.second ?? 0 };
}

/** A zone's offset from UTC at a moment, in minutes (east positive). */
export function offsetMinutes(ms: number, zone: string): number {
  const p = partsIn(ms, zone);
  const asUtc = Date.UTC(p.year, p.month - 1, p.day, p.hour, p.minute, p.second);
  return Math.round((asUtc - Math.floor(ms / 1000) * 1000) / 60_000);
}

/** A zone's offset as people write it: `UTC+8`, `UTC−3:30`, `UTC`. */
export function offsetLabel(zone: string, at = Date.now()): string {
  let minutes: number;
  try {
    minutes = offsetMinutes(at, zone);
  } catch {
    return "";
  }
  if (!minutes) return "UTC";
  const sign = minutes > 0 ? "+" : "−";
  const abs = Math.abs(minutes);
  return `UTC${sign}${Math.floor(abs / 60)}${abs % 60 ? `:${pad(abs % 60)}` : ""}`;
}

/** A moment as the wall clock of a zone. */
export function wallClockOf(ms: number, zone: string): WallClock {
  const p = partsIn(ms, zone);
  return { date: `${p.year}-${pad(p.month)}-${pad(p.day)}`, time: `${pad(p.hour)}:${pad(p.minute)}` };
}

/** `YYYY-MM-DD` and `HH:MM`, read; null when either is not one. */
export function parseWall(date: string, time: string): { y: number; mo: number; d: number; h: number; mi: number } | null {
  const dm = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date.trim());
  const tm = /^(\d{1,2}):(\d{2})$/.exec(time.trim());
  if (!dm || !tm) return null;
  const [y, mo, d, h, mi] = [Number(dm[1]), Number(dm[2]), Number(dm[3]), Number(tm[1]), Number(tm[2])];
  if (mo < 1 || mo > 12 || d < 1 || h > 23 || mi > 59) return null;
  // A day the month does not have (2-30) is not a date.
  if (new Date(Date.UTC(y, mo - 1, d)).getUTCDate() !== d) return null;
  return { y, mo, d, h, mi };
}

/**
 * The moment a zone's wall clock shows a date and time; null when it is not one. A time shown twice
 * (the clocks put back) is the first; one a daylight-saving change skips comes out past the gap, as
 * the scheduler moves it.
 */
export function momentOf(date: string, time: string, zone: string): number | null {
  const w = parseWall(date, time);
  if (!w) return null;
  const guess = Date.UTC(w.y, w.mo - 1, w.d, w.h, w.mi);
  try {
    // The moment under each offset the zone has around then: the one before a change, and after.
    const before = guess - offsetMinutes(guess - 86_400_000, zone) * 60_000;
    const after = guess - offsetMinutes(guess + 86_400_000, zone) * 60_000;
    const shows = (ms: number) => {
      const c = wallClockOf(ms, zone);
      return c.date === date.trim() && c.time === time.trim().padStart(5, "0");
    };
    const exact = [before, after].filter(shows);
    return exact.length ? Math.min(...exact) : Math.max(before, after);
  } catch {
    return null;
  }
}

/** `HH:MM`, valid. */
export function isTime(time: string): boolean {
  return parseWall("2000-01-01", time) !== null;
}

/**
 * A moment as the page writes it: the date (with the weekday) and time in the reader's zone, or in
 * `zone` when given — "11月17日周二 09:00", "Tue, Nov 17, 09:00".
 */
export function formatWhen(ms: number, zone?: string): string {
  try {
    return new Intl.DateTimeFormat(currentLocale(), { month: "short", day: "numeric", weekday: "short", hour: "2-digit", minute: "2-digit", hourCycle: "h23", timeZone: zone }).format(new Date(ms));
  } catch {
    return new Date(ms).toISOString();
  }
}

/** How far a moment is from now, in words: "in 5 min", "3 小时后"; one past, how long ago. */
export function relativeTo(ms: number, now = Date.now()): string {
  const secs = (ms - now) / 1000;
  if (secs < 0) {
    const ago = -secs;
    if (ago < 60) return t("time.justNow");
    if (ago < 3600) return t("time.minutes", { count: Math.floor(ago / 60) });
    if (ago < 86400) return t("time.hours", { count: Math.floor(ago / 3600) });
    return t("time.days", { count: Math.floor(ago / 86400) });
  }
  if (secs < 60) return t("schedule.in.moment");
  if (secs < 3600) return t("schedule.in.minutes", { count: Math.ceil(secs / 60) });
  if (secs < 48 * 3600) return t("schedule.in.hours", { count: Math.round(secs / 3600) });
  return t("schedule.in.days", { count: Math.round(secs / 86400) });
}

/** A time to come as the lists write it: when, in the reader's zone, and how far off. */
export function whenText(ms: number, now = Date.now()): string {
  return `${formatWhen(ms)} · ${relativeTo(ms, now)}`;
}
