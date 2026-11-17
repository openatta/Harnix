/**
 * The time-zone picker: a search box over the zones the browser knows — by name, city or offset
 * ("shanghai", "new york", "+8") — and the list it narrows, the zones used lately first. This
 * machine's zone is the default; the one picked is remembered as used lately once a schedule is
 * saved with it (this page's `localStorage`, `harnix.schedule.zones`).
 */

import { useMemo, useState } from "react";
import { useT } from "@harnix/react";

import { allZones, localZone, offsetLabel } from "./time";

const RECENT_KEY = "harnix.schedule.zones";
const RECENT_MAX = 5;

export function recentZones(): string[] {
  try {
    const raw = JSON.parse(globalThis.localStorage?.getItem(RECENT_KEY) ?? "[]") as unknown;
    return Array.isArray(raw) ? raw.filter((z): z is string => typeof z === "string").slice(0, RECENT_MAX) : [];
  } catch {
    return [];
  }
}

export function rememberZone(zone: string): void {
  try {
    const next = [zone, ...recentZones().filter((z) => z !== zone)].slice(0, RECENT_MAX);
    globalThis.localStorage?.setItem(RECENT_KEY, JSON.stringify(next));
  } catch {
    // A page that cannot keep it simply does not remember.
  }
}

/** Whether a zone answers a search: its name or city (`_` read as a space), or its offset. */
export function zoneMatches(zone: string, query: string, at = Date.now()): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  const name = zone.toLowerCase().replace(/_/g, " ");
  if (name.includes(q.replace(/_/g, " "))) return true;
  const offset = offsetLabel(zone, at).toLowerCase().replace("−", "-");
  const qo = q.replace("−", "-").replace(/^utc|^gmt/, "");
  return qo.length > 0 && /^[+-]/.test(qo) && offset.replace(/^utc/, "").startsWith(qo);
}

export function ZonePicker({ value, onChange, invalid }: { value: string; onChange(zone: string): void; invalid?: boolean }) {
  const t = useT();
  const [query, setQuery] = useState("");
  const local = localZone();
  const recent = useMemo(() => recentZones().filter((z) => z !== local), [local]);
  const matches = useMemo(() => {
    const now = Date.now();
    return allZones().filter((z) => zoneMatches(z, query, now));
  }, [query]);
  const label = (zone: string) => `${zone.replace(/_/g, " ")} (${offsetLabel(zone)})`;
  // The zone picked is always among the options, whatever the search says.
  const listed = new Set([local, ...recent, ...matches]);
  return (
    <div className="hx-sched-zone">
      <input
        className="hx-sched-input"
        type="search"
        data-testid="schedule-zone-search"
        aria-label={t("schedule.editor.zoneSearch")}
        placeholder={t("schedule.editor.zoneSearch")}
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
        }}
      />
      <select data-testid="schedule-zone" aria-label={t("schedule.editor.zone")} aria-invalid={invalid || undefined} value={value} onChange={(e) => onChange(e.target.value)}>
        {!listed.has(value) ? <option value={value}>{value}</option> : null}
        <optgroup label={t("schedule.editor.zoneRecent")}>
          <option value={local}>{label(local)}</option>
          {recent.map((z) => (
            <option key={`r:${z}`} value={z}>
              {label(z)}
            </option>
          ))}
        </optgroup>
        <optgroup label={t("schedule.editor.zoneAll")}>
          {matches
            .filter((z) => z !== local && !recent.includes(z))
            .map((z) => (
              <option key={z} value={z}>
                {label(z)}
              </option>
            ))}
        </optgroup>
      </select>
      <div className="hint" data-testid="schedule-zone-note">
        {value === local ? t("schedule.editor.zoneLocal", { offset: offsetLabel(value) }) : t("schedule.editor.zoneOther", { offset: offsetLabel(value) })}
      </div>
    </div>
  );
}
