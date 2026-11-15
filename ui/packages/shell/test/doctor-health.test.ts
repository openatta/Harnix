/** The doctor view reads Agix's `health` block: a degraded check is a warning, anything unknown a failure. */

import { describe, expect, it } from "vitest";

import { healthTone } from "../src/settings.tsx";

describe("healthTone", () => {
  it("maps the three statuses Agix reports", () => {
    expect(healthTone("ok")).toBe("ok");
    expect(healthTone("degraded")).toBe("warn");
    expect(healthTone("failing")).toBe("err");
  });

  it("reads a missing or unknown status as a failure", () => {
    expect(healthTone(undefined)).toBe("err");
    expect(healthTone("surprise")).toBe("err");
  });
});
