/** Which way the store's newest version is from the installed one: the packages' settings section offers going back only by name. */

import { describe, expect, it } from "vitest";

import { older } from "../src/settings.tsx";

describe("older", () => {
  it("compares part by part as numbers", () => {
    expect(older("0.1.0", "0.2.0")).toBe(true);
    expect(older("0.2.0", "0.1.0")).toBe(false);
    expect(older("0.9.0", "0.10.0")).toBe(true);
    expect(older("1.0", "1.0.1")).toBe(true);
  });
  it("is not older when it is the same", () => {
    expect(older("0.2.0", "0.2.0")).toBe(false);
    expect(older("0.2.0-rc1", "0.2.0")).toBe(false);
  });
});
