/**
 * What the envelope rows say changed between two requests — the one judgement in the request
 * envelope that is the interface's rather than the recording's.
 */

import { describe, expect, it } from "vitest";
import { change, type Header } from "../src/envelope";

const base: Header = {
  model: "m",
  max_tokens: 100,
  system: [{ role: "system", text: "you are" }],
  tools: [{ name: "Bash", source: "builtin" }],
};

describe("envelope changes", () => {
  it("calls the first one initial", () => {
    expect(change(null, base)).toBe("initial");
  });
  it("tells system, tools, both, and the call configuration apart", () => {
    expect(change(base, { ...base, system: [{ role: "system", text: "you were" }] })).toBe("system");
    expect(change(base, { ...base, tools: [] })).toBe("tools");
    expect(change(base, { ...base, system: [], tools: [] })).toBe("systemAndTools");
    expect(change(base, { ...base, model: "fallback" })).toBe("config");
  });
});
