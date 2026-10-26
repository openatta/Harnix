/**
 * The shell asks every contribution point. A point registered into and never read is one where a
 * package's contribution silently does nothing — five of seven points were once in that state with
 * every test green.
 */

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { POINTS } from "@harnix/react";

describe("contribution points", () => {
  it("are each read somewhere in the shell", () => {
    const dir = join(process.cwd(), "src");
    const source = readdirSync(dir)
      .filter((f) => /\.tsx?$/.test(f) && f !== "builtins.tsx")
      .map((f) => readFileSync(join(dir, f), "utf8"))
      .join("\n");
    const unread = POINTS.filter((point) => {
      if (point === "sidebar.group") return !source.includes("registry.grouping()");
      return !new RegExp(`(at|claim)\\("${point.replace(".", "\\.")}"`).test(source);
    });
    expect(unread).toEqual([]);
  });
});
