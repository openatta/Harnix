/** Schedules as the scheduler answers them, for the tests. */

import type { Schedule } from "@harnix/core";

export const ROOT = "/work/alpha";

export function schedule(over: Partial<Schedule> = {}): Schedule {
  return {
    id: "sch-1",
    title: "Morning check",
    enabled: true,
    timing: { kind: "daily", time: "09:00" },
    time_zone: "Asia/Shanghai",
    target: { kind: "session", instance: "main", session_id: "s1", prompt: "Say exactly: fixture ready" },
    missed: "catch_up_latest",
    concurrency: "skip",
    space: "sessions",
    created_by: "device:d1",
    created_at: 1,
    revision: 3,
    next_fire: Date.parse("2026-11-17T01:00:00Z"),
    ...over,
  };
}
