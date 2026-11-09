/**
 * How the shell writes times, sizes and counts: one place, so a figure reads the same in the
 * sidebar, the settings and the system space.
 */

import { currentLocale } from "@harnix/react";

/** How long ago `ms` was, short: `42s`, `5m`, `2h05m`. */
export function since(ms: number): string {
  const secs = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m`;
  return `${Math.floor(secs / 3600)}h${String(Math.floor((secs % 3600) / 60)).padStart(2, "0")}m`;
}

/** How long ago an ISO time was, in words ("just now", "5 min"), then the date. */
export function ago(iso: string | undefined, t: (k: string, v?: Record<string, unknown>) => string): string {
  if (!iso) return "";
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return "";
  const secs = Math.max(0, (Date.now() - then) / 1000);
  if (secs < 60) return t("time.justNow");
  if (secs < 3600) return t("time.minutes", { count: Math.floor(secs / 60) });
  if (secs < 86400) return t("time.hours", { count: Math.floor(secs / 3600) });
  if (secs < 86400 * 30) return t("time.days", { count: Math.floor(secs / 86400) });
  return new Date(then).toLocaleDateString(currentLocale());
}

/** A count of seconds as a clock: `4:05`, `1:04:05`. */
export function clock(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return h ? `${h}:${String(m).padStart(2, "0")}:${String(secs % 60).padStart(2, "0")}` : `${m}:${String(secs % 60).padStart(2, "0")}`;
}

/** Milliseconds, as long as they need to be: `640ms`, `1.5s`, `2m05s`. */
export function millis(n: number): string {
  if (n < 1000) return `${n}ms`;
  const secs = n / 1000;
  return secs < 60 ? `${secs.toFixed(1)}s` : `${Math.floor(secs / 60)}m${String(Math.round(secs % 60)).padStart(2, "0")}s`;
}

/** A count, in thousands past a thousand: `980`, `1.2k`. */
export function compact(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
}

/** A file's size: bytes, KB or MB. */
export function fileSize(bytes: number | undefined): string {
  const n = bytes ?? 0;
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1048576).toFixed(1)} MB`;
}

/** Memory or storage against its limit, always in MB: two places under one. */
export function megabytes(bytes?: number): string {
  return bytes == null ? "—" : `${(bytes / 1048576).toFixed(bytes < 1048576 ? 2 : 0)} MB`;
}
