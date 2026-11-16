/**
 * The shell asks every contribution point. A point registered into and never read is one where a
 * package's contribution silently does nothing — five of seven points were once in that state with
 * every test green.
 */

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { POINTS } from "@harnix/react";

/** Every source file under `dir`, read. */
function sources(dir: string, skip: string[] = []): string {
  return readdirSync(dir, { recursive: true, encoding: "utf8" })
    .filter((f) => /\.tsx?$/.test(f) && !skip.includes(f))
    .map((f) => readFileSync(join(dir, f), "utf8"))
    .join("\n");
}

describe("contribution points", () => {
  it("are each read somewhere in the shell", () => {
    const source = sources(join(process.cwd(), "src"), ["builtins.tsx"]);
    // The session list's grouping is asked by the column that lists a space's sessions by group:
    // the session space's, which is the Harnix app's UI package.
    const grouping = sources(join(process.cwd(), "..", "harnix-app", "src"));
    const unread = POINTS.filter((point) => {
      if (point === "sidebar.group") return !grouping.includes(".grouping()");
      if (point === "session.action") return !source.includes("registry.actions(");
      return !new RegExp(`(at|claim)\\("${point.replace(".", "\\.")}"`).test(source);
    });
    expect(unread).toEqual([]);
  });

  it("asks for session actions at each of the four places", () => {
    const read = (f: string) => readFileSync(join(process.cwd(), "src", f), "utf8");
    // The title bar, the tool row (drawn by the built-in row), the session menu.
    expect(read("Shell.tsx")).toContain('place="header"');
    expect(read("builtins.tsx")).toContain('place="tool"');
    // A tool row's argument asks the actions at `arg`.
    expect(read("builtins.tsx")).toContain("<ActionArg");
    expect(read("actions.tsx")).toContain('asked(app, "arg", target)');
    expect(read("dialogs.tsx")).toContain("actionMenuItems(app, target)");
  });
});
