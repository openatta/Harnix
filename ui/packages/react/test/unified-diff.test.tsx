/**
 * A unified diff (a turn's change as the engine reports it) drawn the way a tool row's diff is:
 * numbered by the file's own lines, a gap between hunks, the headers left out.
 */

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render } from "@testing-library/react";

import { UnifiedDiffView, unifiedDiffRows } from "../src/index";

afterEach(cleanup);

const DIFF = [
  "--- a/src/a.rs",
  "+++ b/src/a.rs",
  "@@ -3,3 +3,3 @@ fn main() {",
  " keep",
  "--- removed, though it reads like a header",
  "+++ added, the same",
  " tail",
  "@@ -40 +40,2 @@",
  "-x",
  "+y",
  "+z",
  "\\ No newline at end of file",
  "",
].join("\n");

describe("a unified diff", () => {
  it("is numbered by the file's lines, hunk by hunk, its headers not rows", () => {
    expect(unifiedDiffRows(DIFF)).toEqual([
      { op: "same", text: "keep", oldNo: 3, newNo: 3 },
      { op: "del", text: "-- removed, though it reads like a header", oldNo: 4 },
      { op: "add", text: "++ added, the same", newNo: 4 },
      { op: "same", text: "tail", oldNo: 5, newNo: 5 },
      { op: "gap", text: "…" },
      { op: "del", text: "x", oldNo: 40 },
      { op: "add", text: "y", newNo: 40 },
      { op: "add", text: "z", newNo: 41 },
    ]);
  });

  it("reads two diffs of one file one after the other", () => {
    const twice = "--- /dev/null\n+++ b/n.txt\n@@ -0,0 +1 @@\n+one\n--- a/n.txt\n+++ b/n.txt\n@@ -1 +1 @@\n-one\n+two\n";
    expect(unifiedDiffRows(twice).map((r) => r.op)).toEqual(["add", "gap", "del", "add"]);
  });

  it("is drawn with the tool row's diff lines", () => {
    const page = render(<UnifiedDiffView path="src/a.rs" diff={DIFF} testId="changes-diff" />);
    expect(page.getByTestId("changes-diff").textContent).toContain("src/a.rs");
    expect(page.getAllByTestId("conv-diff-line").map((el) => el.dataset.op)).toEqual(["same", "del", "add", "same", "del", "add", "add"]);
  });
});
