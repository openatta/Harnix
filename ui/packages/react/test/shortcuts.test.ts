/**
 * Keys as commands' (`shortcuts.ts`): how a key is written and read back on each platform, how it is
 * shown, which keys a command may not have, which commands share one, and which a press runs.
 */

import { describe, expect, it } from "vitest";
import { Registry } from "../src/contrib";
import {
  chordId,
  chordOf,
  defaultKeys,
  keyLabel,
  keysProblem,
  parseKeys,
  pickShortcut,
  shortcutConflicts,
  writeKeys,
  type ShortcutBinding,
} from "../src/shortcuts";

const press = (init: Partial<KeyboardEventInit> & { key: string }) => new KeyboardEvent("keydown", init);

describe("a key, written and read", () => {
  it("reads Mod as ⌘ on a Mac and Ctrl elsewhere, and the other modifiers as themselves", () => {
    expect(parseKeys("Mod+K", "mac")).toEqual({ ctrl: false, alt: false, shift: false, meta: true, code: "KeyK" });
    expect(parseKeys("Mod+K", "other")).toEqual({ ctrl: true, alt: false, shift: false, meta: false, code: "KeyK" });
    expect(parseKeys("ctrl+alt+shift+cmd+1", "other")).toEqual({ ctrl: true, alt: true, shift: true, meta: true, code: "Digit1" });
    expect(parseKeys("Option+Up", "mac")?.code).toBe("ArrowUp");
    expect(parseKeys("Mod+\\", "mac")?.code).toBe("Backslash");
    expect(parseKeys("Mod+/", "mac")?.code).toBe("Slash");
    expect(parseKeys("Mod+KeyK", "mac")?.code).toBe("KeyK");
    expect(parseKeys("F5", "mac")?.code).toBe("F5");
  });

  it("refuses what is not a key: nothing, an unknown key or modifier, a modifier twice", () => {
    for (const text of ["", "Mod+", "Mod+Nope", "Hyper+K", "Mod+Ctrl+Ctrl+K", "Mod++"]) expect(parseKeys(text, "other"), text).toBeNull();
  });

  it("writes a key so it reads back the same on either platform", () => {
    for (const text of ["Mod+K", "Alt+ArrowUp", "Mod+Shift+\\", "Ctrl+Alt+/", "Meta+F12", "Mod+,"]) {
      for (const platform of ["mac", "other"] as const) {
        const chord = parseKeys(text, platform)!;
        expect(parseKeys(writeKeys(chord), platform), `${text} ${platform}`).toEqual(chord);
      }
    }
    expect(writeKeys(parseKeys("Mod+Shift+\\", "mac")!)).toBe("Shift+Meta+\\");
  });

  it("is shown the platform's way", () => {
    expect(keyLabel(parseKeys("Mod+K", "mac")!, "mac")).toBe("⌘K");
    expect(keyLabel(parseKeys("Mod+Shift+Alt+Up", "mac")!, "mac")).toBe("⌥⇧⌘↑");
    expect(keyLabel(parseKeys("Mod+K", "other")!, "other")).toBe("Ctrl+K");
    expect(keyLabel(parseKeys("Alt+Down", "other")!, "other")).toBe("Alt+↓");
    expect(keyLabel(parseKeys("Mod+\\", "other")!, "other")).toBe("Ctrl+\\");
  });

  it("is the default of the platform it is given for, one for all or one each", () => {
    expect(chordId(defaultKeys("Mod+K", "mac")!)).toBe("Meta+KeyK");
    expect(chordId(defaultKeys("Mod+K", "other")!)).toBe("Ctrl+KeyK");
    const each = { mac: "Ctrl+Alt+K", other: null };
    expect(chordId(defaultKeys(each, "mac")!)).toBe("Ctrl+Alt+KeyK");
    expect(defaultKeys(each, "other")).toBeNull();
    expect(defaultKeys(undefined, "mac")).toBeNull();
  });

  it("is what a press is: by the physical key, or the character's key when the event has no code", () => {
    // ⌥K on a Mac makes "˚": the key is still K.
    expect(chordOf(press({ key: "˚", code: "KeyK", altKey: true }))).toEqual({ ctrl: false, alt: true, shift: false, meta: false, code: "KeyK" });
    expect(chordOf(press({ key: "k", metaKey: true }))?.code).toBe("KeyK");
    expect(chordOf(press({ key: "\\", ctrlKey: true }))?.code).toBe("Backslash");
    expect(chordOf(press({ key: "Meta", code: "MetaLeft", metaKey: true }))).toBeNull();
  });

  it("needs Ctrl, ⌘ or ⌥ unless it is a function key, and leaves editing and the browser's keys alone", () => {
    expect(keysProblem(parseKeys("K", "mac")!, "mac")).toBe("shortcuts.problem.modifier");
    expect(keysProblem(parseKeys("Shift+K", "mac")!, "mac")).toBe("shortcuts.problem.modifier");
    expect(keysProblem(parseKeys("F2", "mac")!, "mac")).toBeNull();
    expect(keysProblem(parseKeys("Mod+C", "mac")!, "mac")).toBe("shortcuts.problem.reserved");
    expect(keysProblem(parseKeys("Ctrl+C", "other")!, "other")).toBe("shortcuts.problem.reserved");
    // Ctrl+C on a Mac is not copy.
    expect(keysProblem(parseKeys("Ctrl+C", "mac")!, "mac")).toBeNull();
    expect(keysProblem(parseKeys("Mod+Escape", "mac")!, "mac")).toBe("shortcuts.problem.reserved");
  });
});

const bind = (id: string, keys: string | null, scope: ShortcutBinding["scope"] = "global"): ShortcutBinding => ({ id, scope, chord: keys ? parseKeys(keys, "mac") : null });

describe("commands and their keys", () => {
  it("conflict when they share a key where both are in force: a global one is everywhere, the composer and the right column never meet", () => {
    const conflicts = shortcutConflicts([
      bind("a", "Mod+K"),
      bind("b", "Meta+K"),
      bind("c", "Alt+ArrowUp", "composer"),
      bind("d", "Alt+ArrowUp", "sidebar"),
      bind("e", "Mod+J", "composer"),
      bind("f", "Mod+J"),
      bind("g", null),
    ]);
    expect(Object.fromEntries(conflicts)).toEqual({ a: ["b"], b: ["a"], e: ["f"], f: ["e"] });
  });

  it("a press runs the focused place's command before a global one, then the first registered", () => {
    const all = [bind("first", "Mod+K"), bind("second", "Mod+K"), bind("composer", "Mod+K", "composer"), bind("side", "Alt+Up", "sidebar")];
    const k = parseKeys("Mod+K", "mac")!;
    expect(pickShortcut(all, k, ["global"])?.id).toBe("first");
    expect(pickShortcut(all, k, ["composer", "global"])?.id).toBe("composer");
    const up = parseKeys("Alt+ArrowUp", "mac")!;
    expect(pickShortcut(all, up, ["global"])).toBeNull();
    expect(pickShortcut(all, up, ["sidebar", "global"])?.id).toBe("side");
  });
});

describe("the shortcut point", () => {
  it("takes a command with a title and a run; refuses a bad scope, keys that are not keys, and an id already taken", () => {
    const r = new Registry();
    const run = () => undefined;
    expect(r.register("shortcut", { id: "a.go", title: () => "Go", keys: "Mod+G", run }, "a")).toBe(true);
    expect(r.register("shortcut", { id: "a.each", title: () => "Each", keys: { mac: "Ctrl+E", other: null }, scope: "composer", run }, "a")).toBe(true);
    expect(r.register("shortcut", { id: "a.none", title: () => "None", run }, "a")).toBe(true);
    expect(r.register("shortcut", { id: "a.where", title: () => "x", scope: "nowhere" as never, run }, "a")).toBe(false);
    expect(r.register("shortcut", { id: "a.bad", title: () => "x", keys: "Mod+Nope", run }, "a")).toBe(false);
    expect(r.register("shortcut", { id: "a.go", title: () => "again", run }, "b")).toBe(false);
    expect(r.register("shortcut", { id: "a.norun", title: () => "x" } as never, "a")).toBe(false);
    expect(r.at("shortcut").map((c) => c.id)).toEqual(["a.go", "a.each", "a.none"]);
    expect(r.refusals().map((x) => `${x.id}: ${x.reason}`)).toEqual([
      'a.where: a shortcut\'s scope is global, composer, sidebar, not "nowhere"',
      'a.bad: a shortcut\'s keys are written like "Mod+K", not "Mod+Nope"',
      "a.go: `a.go` is already a command (a)",
      "a.norun: missing run",
    ]);
    // Withdrawn with its owner, and its id free again.
    r.withdraw("a");
    expect(r.at("shortcut")).toEqual([]);
    expect(r.register("shortcut", { id: "a.go", title: () => "again", run }, "b")).toBe(true);
  });
});
