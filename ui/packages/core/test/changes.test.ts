/**
 * A turn's changes as the hub files them in the session's log: read in one place, the changes card
 * and the changes tab alike.
 */

import { describe, expect, it } from "vitest";
import { CHANGES_EVENT, CHANGES_NS, parseTurnChanges, projectRelative } from "../src/protocol/index.ts";

describe("a turn's changes", () => {
  it("reads the hub's entry, and nothing else", () => {
    const entry = {
      ns: CHANGES_NS,
      event: CHANGES_EVENT,
      payload: {
        turn_id: "t1",
        added: 4,
        removed: 1,
        more: 2,
        files: [
          { path: "/w/a.rs", op: "create", added: 3, removed: 0, bytes: 30, diff: "--- /dev/null\n+++ b/a.rs\n" },
          { path: "/w/b.bin", op: "modify", added: 1, removed: 1, omitted: "binary" },
          { op: "modify" },
        ],
      },
    };
    expect(parseTurnChanges(entry)).toEqual({
      turnId: "t1",
      added: 4,
      removed: 1,
      more: 2,
      files: [
        { path: "/w/a.rs", op: "create", added: 3, removed: 0, bytes: 30, diff: "--- /dev/null\n+++ b/a.rs\n" },
        { path: "/w/b.bin", op: "modify", added: 1, removed: 1, omitted: "binary" },
      ],
    });
    expect(parseTurnChanges({ ns: "harnix.audit", event: "turn", payload: entry.payload })).toBeNull();
    expect(parseTurnChanges({ ns: CHANGES_NS, event: CHANGES_EVENT, payload: { files: [] } })).toBeNull();
    expect(parseTurnChanges(null)).toBeNull();
  });

  it("names a path by the project when it is in it", () => {
    expect(projectRelative("/w/p/src/a.rs", "/w/p")).toBe("src/a.rs");
    expect(projectRelative("/w/p/src/a.rs", "/w/p/")).toBe("src/a.rs");
    expect(projectRelative("/w/px/a.rs", "/w/p")).toBe("/w/px/a.rs");
    expect(projectRelative("/w/a.rs", null)).toBe("/w/a.rs");
  });
});
