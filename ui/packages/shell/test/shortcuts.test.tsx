/**
 * The shortcut service (`shortcuts.tsx`): the keys in force — a platform's default or the person's
 * own, kept in this browser — which command a press runs where the focus is, the keys a frame hands
 * over; and the settings section that records a key, refuses one another command has, and restores
 * the defaults.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider, Registry, chordId, parseKeys, type ShortcutCommand } from "@harnix/react";
import { pressChord } from "@harnix/ext-sdk/protocol";

import { App } from "../src/app";
import { Shell } from "../src/Shell";
import { SHORTCUT_PREFIX, Shortcuts, ShortcutsSection, registerShellShortcuts, scopesAt } from "../src/shortcuts";
import { scriptedApp } from "./scripted";

afterEach(() => {
  cleanup();
  localStorage.clear();
  document.body.innerHTML = "";
});

const command = (id: string, over: Partial<ShortcutCommand> = {}): ShortcutCommand => ({ id, title: () => id, run: () => undefined, ...over });

function service(platform: "mac" | "other" = "mac") {
  const registry = new Registry();
  return { registry, shortcuts: new Shortcuts(registry, { platform, storage: localStorage }) };
}

describe("the keys in force", () => {
  it("are each command's default on this platform until the person picks another, kept under harnix.shortcuts.<id>", async () => {
    const { registry, shortcuts } = service("mac");
    registry.register("shortcut", command("a.go", { keys: "Mod+G" }), "a");
    registry.register("shortcut", command("a.each", { keys: { mac: "Ctrl+E", other: "Alt+E" } }), "a");
    registry.register("shortcut", command("a.none"), "a");
    expect(shortcuts.label("a.go")).toBe("⌘G");
    expect(shortcuts.label("a.each")).toBe("⌃E");
    expect(shortcuts.label("a.none")).toBeNull();
    expect(new Shortcuts(registry, { platform: "other" }).label("a.each")).toBe("Alt+E");

    // A local field store's shape: a text field per command, its default the platform's key.
    const store = shortcuts.store();
    const doc = await store.load();
    expect(doc.fields.map((f) => [f.key, f.type, f.default, f.effective])).toEqual([
      ["a.go", "text", "Meta+G", "Meta+G"],
      ["a.each", "text", "Ctrl+E", "Ctrl+E"],
      ["a.none", "text", undefined, undefined],
    ]);
    await store.save({ "a.go": "Meta+J" }, doc.revision);
    expect(localStorage.getItem(`${SHORTCUT_PREFIX}a.go`)).toBe('"Meta+J"');
    expect(shortcuts.label("a.go")).toBe("⌘J");
    expect(shortcuts.rows().find((r) => r.id === "a.go")).toMatchObject({ custom: true, owner: "a" });
    // Written over values it did not read: refused.
    await expect(store.save({ "a.go": "Meta+H" }, doc.revision)).rejects.toMatchObject({ code: expect.any(Number) });
    await store.save({ "a.go": null }, (await store.load()).revision);
    expect(localStorage.getItem(`${SHORTCUT_PREFIX}a.go`)).toBeNull();
    expect(shortcuts.label("a.go")).toBe("⌘G");
  });

  it("follow another tab's change, and ignore a stored key that is not one", () => {
    const { registry, shortcuts } = service("mac");
    registry.register("shortcut", command("a.go", { keys: "Mod+G" }), "a");
    const off = shortcuts.follow();
    try {
      localStorage.setItem(`${SHORTCUT_PREFIX}a.go`, '"Alt+G"');
      window.dispatchEvent(new StorageEvent("storage", { key: `${SHORTCUT_PREFIX}a.go` }));
      expect(shortcuts.label("a.go")).toBe("⌥G");
      localStorage.setItem(`${SHORTCUT_PREFIX}a.go`, '"Mod+Nope"');
      window.dispatchEvent(new StorageEvent("storage", { key: `${SHORTCUT_PREFIX}a.go` }));
      expect(shortcuts.label("a.go")).toBe("⌘G");
    } finally {
      off();
    }
  });

  it("say which commands share a key, refuse a key another has or the page leaves alone, and go with their owner", () => {
    const { registry, shortcuts } = service("mac");
    registry.register("shortcut", command("a.one", { keys: "Mod+K" }), "a");
    registry.register("shortcut", command("b.two", { keys: "Mod+K" }), "b");
    registry.register("shortcut", command("b.three", { keys: "Alt+ArrowUp", scope: "composer" }), "b");
    registry.register("shortcut", command("b.four", { scope: "sidebar" }), "b");
    expect(shortcuts.rows().map((r) => [r.id, r.conflicts])).toEqual([
      ["a.one", ["b.two"]],
      ["b.two", ["a.one"]],
      ["b.three", []],
      ["b.four", []],
    ]);
    const key = (text: string) => parseKeys(text, "mac")!;
    expect(shortcuts.problem("b.four", key("Alt+Up"))).toBeNull(); // the composer's and the right column's never meet
    expect(shortcuts.problem("a.one", key("Alt+Up"))).toEqual({ key: "shortcuts.problem.conflict", vars: { commands: "b.three" } });
    expect(shortcuts.problem("b.four", key("Mod+C"))).toEqual({ key: "shortcuts.problem.reserved" });
    expect(shortcuts.problem("b.four", key("J"))).toEqual({ key: "shortcuts.problem.modifier" });
    registry.withdraw("b");
    expect(shortcuts.rows().map((r) => [r.id, r.conflicts])).toEqual([["a.one", []]]);
  });
});

describe("a press", () => {
  function page() {
    const { app } = scriptedApp();
    registerShellShortcuts(app);
    const ran: string[] = [];
    const add = (id: string, over: Partial<ShortcutCommand>) => app.registry.register("shortcut", command(id, { run: () => void ran.push(id), ...over }), "pkg");
    render(
      <HarnixUiProvider client={app.conn as HarnixClient}>
        <Shell app={app} />
      </HarnixUiProvider>,
    );
    return { app, ran, add };
  }

  const pressAt = (target: EventTarget, init: KeyboardEventInit) => {
    const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
    act(() => {
      target.dispatchEvent(event);
    });
    return event.defaultPrevented;
  };

  it("runs the focused place's command before a global one, and a place's command only there", () => {
    const { ran, add } = page();
    add("pkg.global", { keys: "Alt+ArrowUp" });
    add("pkg.composer", { keys: "Alt+ArrowUp", scope: "composer" });
    add("pkg.side", { keys: "Alt+J", scope: "sidebar" });
    const composer = document.body.appendChild(document.createElement("div"));
    composer.setAttribute("data-testid", "conv-composer");
    const input = composer.appendChild(document.createElement("textarea"));
    expect(scopesAt(input)).toEqual(["composer", "global"]);
    expect(pressAt(input, { key: "ArrowUp", code: "ArrowUp", altKey: true })).toBe(true);
    expect(pressAt(document.body, { key: "ArrowUp", code: "ArrowUp", altKey: true })).toBe(true);
    // The right column's command is not anywhere else.
    expect(pressAt(document.body, { key: "j", code: "KeyJ", altKey: true })).toBe(false);
    const inSide = document.getElementById("details")!.appendChild(document.createElement("button"));
    expect(scopesAt(inSide)).toEqual(["sidebar", "global"]);
    expect(pressAt(inSide, { key: "j", code: "KeyJ", altKey: true })).toBe(true);
    expect(ran).toEqual(["pkg.composer", "pkg.global", "pkg.side"]);
  });

  it("leaves typing alone, skips a command not enabled now, and tells a failure", async () => {
    const { app, ran, add } = page();
    add("pkg.f2", { keys: "F2" });
    add("pkg.off", { keys: "Alt+O", enabled: () => false });
    add("pkg.bad", {
      keys: "Alt+B",
      run: () => {
        throw new Error("nope");
      },
    });
    const field = document.body.appendChild(document.createElement("input"));
    expect(pressAt(field, { key: "F2", code: "F2" })).toBe(true);
    expect(pressAt(document.body, { key: "o", code: "KeyO", altKey: true })).toBe(false);
    expect(pressAt(document.body, { key: "b", code: "KeyB", altKey: true })).toBe(true);
    expect(ran).toEqual(["pkg.f2"]);
    expect(app.snapshot.banner?.text).toBe("nope");
  });

  it("runs the shell's own keys as the person rebound them, and not while a dialog is open", () => {
    const { app } = page();
    const sidebar = () => app.snapshot.sidebar;
    const before = sidebar();
    // Not a Mac: Mod is Ctrl.
    expect(pressAt(window, { key: "\\", ctrlKey: true })).toBe(true);
    expect(sidebar()).not.toBe(before);
    localStorage.setItem(`${SHORTCUT_PREFIX}shell.sidebar.toggle`, '"Ctrl+Alt+S"');
    act(() => app.shortcuts.reload());
    expect(pressAt(window, { key: "\\", ctrlKey: true })).toBe(false);
    expect(pressAt(window, { key: "s", code: "KeyS", ctrlKey: true, altKey: true })).toBe(true);
    expect(sidebar()).toBe(before);
    act(() => app.openModal({ title: "m", content: null }));
    expect(pressAt(window, { key: "s", code: "KeyS", ctrlKey: true, altKey: true })).toBe(false);
  });
});

describe("a frame's keys", () => {
  it("are the global keys in force, named as the frame names a press", () => {
    const app = new App({ status: "open", onNotification: () => () => undefined, onStatus: () => () => undefined } as unknown as HarnixClient, new Registry());
    registerShellShortcuts(app);
    app.registry.register("shortcut", command("pkg.composer", { keys: "Alt+ArrowUp", scope: "composer" }), "pkg");
    const keys = app.shortcuts.globalKeys();
    // Not a Mac here: Ctrl.
    expect(keys).toEqual(["Ctrl+KeyK", "Ctrl+Backslash", "Ctrl+Slash"]);
    expect(pressChord({ key: "k", code: "KeyK", ctrl: true, meta: false, alt: false, shift: false })).toBe(keys[0]);
    expect(pressChord({ key: "Ú", code: "KeyU", ctrl: false, meta: true, alt: true, shift: true })).toBe(chordId(parseKeys("Mod+Alt+Shift+U", "mac")!));
    // `[shell] new_session = false`: the new-session key is not the shell's, in a frame either.
    app.set({ hello: { shell: { new_session: false } } as never });
    expect(app.shortcuts.globalKeys()).toEqual(["Ctrl+Backslash", "Ctrl+Slash"]);
  });
});

describe("the settings section", () => {
  function section() {
    const { app } = scriptedApp();
    registerShellShortcuts(app);
    app.registry.register("shortcut", command("pkg.up", { title: () => "Up", keys: "Alt+ArrowLeft", scope: "composer" }), "pkg");
    const view = render(
      <HarnixUiProvider client={app.conn as HarnixClient}>
        <ShortcutsSection app={app} />
      </HarnixUiProvider>,
    );
    const row = (id: string) => view.container.querySelector<HTMLElement>(`[data-testid="shortcut-row"][data-command="${id}"]`)!;
    const keys = (id: string) => row(id).querySelector<HTMLButtonElement>('[data-testid="shortcut-keys"]')!;
    const record = async (id: string, init: KeyboardEventInit) => {
      fireEvent.click(keys(id));
      expect(keys(id).dataset.state).toBe("recording");
      await act(async () => {
        fireEvent.keyDown(keys(id), init);
      });
      await act(async () => {});
    };
    return { app, view, row, keys, record };
  }

  it("lists every command with its owner and key, and records a new key", async () => {
    const { app, view, row, keys, record } = section();
    await act(async () => {});
    expect([...view.container.querySelectorAll<HTMLElement>('[data-testid="shortcut-row"]')].map((r) => r.dataset.command)).toEqual([
      "shell.session.new",
      "shell.sidebar.toggle",
      "shell.composer.focus",
      "shell.details.toggle",
      "shell.settings.open",
      "chat.turn.previous",
      "chat.turn.next",
      "pkg.up",
    ]);
    expect(keys("shell.session.new").textContent).toBe("Ctrl+K");
    expect(keys("shell.details.toggle").dataset.state).toBe("none");
    expect(row("pkg.up").textContent).toContain("pkg");
    await record("shell.session.new", { key: "j", code: "KeyJ", ctrlKey: true });
    expect(keys("shell.session.new").textContent).toBe("Ctrl+J");
    expect(row("shell.session.new").dataset.state).toBe("custom");
    expect(localStorage.getItem(`${SHORTCUT_PREFIX}shell.session.new`)).toBe('"Ctrl+J"');
    expect(app.shortcuts.label("shell.session.new")).toBe("Ctrl+J");
    expect(view.getByTestId("shortcuts-custom").textContent).not.toBe("");
  });

  it("refuses a key another command has, and one the page leaves alone, and keeps the old one; Escape gives up", async () => {
    const { view, row, keys, record } = section();
    await act(async () => {});
    await record("shell.session.new", { key: "/", code: "Slash", ctrlKey: true });
    expect(row("shell.session.new").querySelector('[data-testid="shortcut-warning"]')).not.toBeNull();
    expect(keys("shell.session.new").textContent).toBe("Ctrl+K");
    await record("shell.session.new", { key: "c", code: "KeyC", ctrlKey: true });
    expect(row("shell.session.new").querySelector('[data-testid="shortcut-warning"]')).not.toBeNull();
    expect(keys("shell.session.new").textContent).toBe("Ctrl+K");
    await record("shell.session.new", { key: "Escape", code: "Escape" });
    expect(keys("shell.session.new").dataset.state).toBeUndefined();
    expect(keys("shell.session.new").textContent).toBe("Ctrl+K");
    expect(localStorage.length).toBe(0);
    expect(view.container.querySelector('[data-state="conflict"]')).toBeNull();
  });

  it("restores one key, then all of them after asking", async () => {
    const { view, row, keys, record } = section();
    await act(async () => {});
    await record("shell.session.new", { key: "j", code: "KeyJ", ctrlKey: true });
    await record("shell.details.toggle", { key: "d", code: "KeyD", ctrlKey: true, altKey: true });
    await record("pkg.up", { key: "ArrowRight", code: "ArrowRight", altKey: true });
    expect(keys("shell.details.toggle").textContent).toBe("Ctrl+Alt+D");
    await act(async () => {
      fireEvent.click(row("shell.session.new").querySelector('[data-testid="shortcut-reset"]')!);
    });
    expect(keys("shell.session.new").textContent).toBe("Ctrl+K");
    expect(row("shell.session.new").querySelector('[data-testid="shortcut-reset"]')).toBeNull();
    fireEvent.click(view.getByTestId("shortcuts-reset-all"));
    await act(async () => {
      fireEvent.click(view.getByTestId("shortcuts-reset-all-confirm"));
    });
    expect(keys("shell.details.toggle").dataset.state).toBe("none");
    expect(keys("pkg.up").textContent).toBe("Alt+←");
    expect(Object.keys(localStorage).filter((k) => k.startsWith(SHORTCUT_PREFIX))).toEqual([]);
    expect((view.getByTestId("shortcuts-reset-all") as HTMLButtonElement).disabled).toBe(true);
  });

  it("marks commands that share a key", async () => {
    const { app, view, row } = section();
    act(() => {
      app.registry.register("shortcut", command("pkg.clash", { keys: "Mod+K" }), "pkg");
    });
    await act(async () => {});
    expect(row("pkg.clash").dataset.state).toBe("conflict");
    expect(row("shell.session.new").dataset.state).toBe("conflict");
    expect(view.getAllByTestId("shortcut-conflict")).toHaveLength(2);
  });
});
