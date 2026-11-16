/** Times in a zone with `Intl` alone: wall clock and moment both ways (daylight saving included), offsets as people write them, a zone found by a search, how far off a time is. */

import { describe, expect, it } from "vitest";
import { Registry, setLocale } from "@harnix/react";

import { registerScheduleStrings } from "../src/strings";
import { allZones, isZone, momentOf, offsetLabel, offsetMinutes, parseWall, relativeTo, wallClockOf } from "../src/time";
import { zoneMatches } from "../src/zone";

setLocale("en");
registerScheduleStrings(new Registry());

describe("a zone's wall clock", () => {
  it("both ways, in a zone without daylight saving", () => {
    const at = momentOf("2026-12-01", "08:30", "Asia/Shanghai")!;
    expect(new Date(at).toISOString()).toBe("2026-12-01T00:30:00.000Z");
    expect(wallClockOf(at, "Asia/Shanghai")).toEqual({ date: "2026-12-01", time: "08:30" });
    expect(wallClockOf(at, "UTC")).toEqual({ date: "2026-12-01", time: "00:30" });
  });

  it("across daylight saving: summer and winter offsets; a skipped time comes out past the gap", () => {
    expect(new Date(momentOf("2026-07-01", "09:00", "America/New_York")!).toISOString()).toBe("2026-07-01T13:00:00.000Z");
    expect(new Date(momentOf("2026-12-01", "09:00", "America/New_York")!).toISOString()).toBe("2026-12-01T14:00:00.000Z");
    // 02:30 on 2026-03-08 does not exist in New York: the moment lands an hour on.
    const skipped = momentOf("2026-03-08", "02:30", "America/New_York")!;
    expect(wallClockOf(skipped, "America/New_York").time).toBe("03:30");
    // 01:30 on 2026-11-01 is shown twice: the first, still in summer time.
    expect(new Date(momentOf("2026-11-01", "01:30", "America/New_York")!).toISOString()).toBe("2026-11-01T05:30:00.000Z");
  });

  it("a date or time that is not one", () => {
    expect(parseWall("2026-02-29", "09:00")).toBeNull();
    expect(parseWall("2028-02-29", "09:00")).not.toBeNull();
    expect(parseWall("2026-01-01", "24:00")).toBeNull();
    expect(momentOf("tomorrow", "09:00", "UTC")).toBeNull();
  });
});

describe("offsets and zones", () => {
  it("as people write them", () => {
    const winter = Date.parse("2026-01-15T00:00:00Z");
    expect(offsetMinutes(winter, "Asia/Kolkata")).toBe(330);
    expect(offsetLabel("Asia/Shanghai", winter)).toBe("UTC+8");
    expect(offsetLabel("Asia/Kolkata", winter)).toBe("UTC+5:30");
    expect(offsetLabel("America/St_Johns", winter)).toBe("UTC−3:30");
    expect(offsetLabel("UTC", winter)).toBe("UTC");
  });

  it("the zones the browser knows; one found by its name, its city or its offset", () => {
    expect(allZones()).toContain("UTC");
    expect(allZones()).toContain("Asia/Shanghai");
    expect(isZone("Asia/Shanghai")).toBe(true);
    expect(isZone("Nowhere/Else")).toBe(false);
    const winter = Date.parse("2026-01-15T00:00:00Z");
    expect(zoneMatches("America/New_York", "new york", winter)).toBe(true);
    expect(zoneMatches("Asia/Shanghai", "shang", winter)).toBe(true);
    expect(zoneMatches("Asia/Shanghai", "+8", winter)).toBe(true);
    expect(zoneMatches("Asia/Shanghai", "UTC+8", winter)).toBe(true);
    expect(zoneMatches("Asia/Shanghai", "-5", winter)).toBe(false);
    expect(zoneMatches("Asia/Shanghai", "", winter)).toBe(true);
  });
});

describe("how far off", () => {
  it("a time to come, and one past", () => {
    const now = Date.parse("2026-11-16T00:00:00Z");
    expect(relativeTo(now + 30_000, now)).toBe("in under a minute");
    expect(relativeTo(now + 5 * 60_000, now)).toBe("in 5 min");
    expect(relativeTo(now + 13 * 3600_000, now)).toBe("in 13 h");
    expect(relativeTo(now + 3 * 86400_000, now)).toBe("in 3 days");
    expect(relativeTo(now - 10 * 60_000, now)).toMatch(/10/);
  });
});
