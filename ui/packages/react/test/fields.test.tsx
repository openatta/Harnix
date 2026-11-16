/** Settings drawn from their declarations: what each type accepts, this browser's store, and the form. */

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { CONFLICT } from "@harnix/core";

import { FieldForm, Registry, checkField, isRisky, localFieldStore, setLocale, type FieldDoc, type FieldState, type FieldStore } from "../src/index";

afterEach(cleanup);
setLocale("en");

/** A store over a document held here, which remembers what it was asked to write. */
function memory(doc: FieldDoc, refuse?: (changes: Record<string, unknown>, revision?: string) => unknown) {
  const saved: { changes: Record<string, unknown>; revision?: string }[] = [];
  let loads = 0;
  const store: FieldStore = {
    load: async () => {
      loads += 1;
      return structuredClone(doc);
    },
    save: async (changes, revision) => {
      const refusal = refuse?.(changes, revision);
      if (refusal) throw refusal;
      saved.push({ changes, revision });
    },
  };
  return { store, saved, loads: () => loads, set: (next: FieldDoc) => (doc = next) };
}

const tiers = (global: unknown, scene: unknown, project: unknown) => ({ global, scene, project });
const WRITABLE = ["global", "scene", "project"];

describe("a field's type", () => {
  it("refuses what it does not mean", () => {
    expect(checkField({ key: "a", type: "toggle" }, true)).toBeNull();
    expect(checkField({ key: "a", type: "toggle" }, "true")).not.toBeNull();
    expect(checkField({ key: "a", type: "select", options: ["plan"] }, "plan")).toBeNull();
    expect(checkField({ key: "a", type: "select", options: ["plan"] }, "yolo")).not.toBeNull();
    const count = { key: "n", type: "number" as const, min: 1, max: 1_000_000 };
    expect(checkField(count, 4000)).toBeNull();
    expect(checkField(count, 0)).toMatchObject({ key: "settings.invalid.number", vars: { min: 1, max: 1_000_000 } });
    expect(checkField(count, 4.5)).not.toBeNull();
    expect(checkField({ key: "t", type: "text" }, "  ")).not.toBeNull();
    expect(checkField({ key: "s", type: "secret" }, "sk-1")).toBeNull();
    expect(checkField({ key: "l", type: "list" }, ["a", "b"])).toBeNull();
    expect(checkField({ key: "l", type: "list" }, ["a", ""])).not.toBeNull();
    const rules = { key: "r", type: "rules" as const, columns: [{ key: "tool", required: true }, { key: "action", options: ["allow", "deny"], required: true }] };
    expect(checkField(rules, [{ tool: "Bash", action: "allow" }])).toBeNull();
    expect(checkField(rules, [{ tool: "Bash", action: "maybe" }])).toMatchObject({ vars: { row: 1 } });
    expect(checkField(rules, [{ tool: "Bash", action: "deny" }, { action: "deny" }])).toMatchObject({ vars: { row: 2 } });
    expect(checkField(rules, [{ tool: "Bash", action: "deny", shell: "x" }])).not.toBeNull();
  });

  it("says which values are a risk", () => {
    const mode = { key: "m", type: "select" as const, options: ["default", "bypassPermissions"], risk: { values: ["bypassPermissions"], message: "m" } };
    expect(isRisky(mode, "bypassPermissions")).toBe(true);
    expect(isRisky(mode, "default")).toBe(false);
    expect(isRisky(mode, null)).toBe(false);
    expect(isRisky({ key: "k", type: "secret", risk: { message: "m" } }, "anything")).toBe(true);
  });
});

describe("this browser's preferences", () => {
  function storage(): Storage {
    const m = new Map<string, string>();
    return {
      getItem: (k) => m.get(k) ?? null,
      setItem: (k, v) => void m.set(k, v),
      removeItem: (k) => void m.delete(k),
      clear: () => m.clear(),
      key: (i) => [...m.keys()][i] ?? null,
      get length() {
        return m.size;
      },
    };
  }

  it("are their defaults until set, and their defaults again when given up", async () => {
    const local = storage();
    const store = localFieldStore([{ key: "size", type: "number", min: 10, max: 24, default: 14 }], { prefix: "p.", storage: local });
    expect((await store.load()).fields[0]!.effective).toBe(14);
    await store.save({ size: 16 });
    expect(local.getItem("p.size")).toBe("16");
    expect((await store.load()).fields[0]!.effective).toBe(16);
    await store.save({ size: null });
    expect(local.getItem("p.size")).toBeNull();
    expect((await store.load()).fields[0]!.effective).toBe(14);
    await expect(store.save({ size: 99 })).rejects.toMatchObject({ data: { errors: [{ key: "size" }] } });
    await expect(store.save({ other: 1 })).rejects.toMatchObject({ data: { errors: [{ key: "other" }] } });
  });

  it("refuse a write over a value another tab changed", async () => {
    const local = storage();
    const store = localFieldStore([{ key: "size", type: "number", default: 14 }], { prefix: "p.", storage: local });
    const read = await store.load();
    local.setItem("p.size", "18");
    await expect(store.save({ size: 16 }, read.revision)).rejects.toMatchObject({ code: CONFLICT });
    expect(local.getItem("p.size")).toBe("18");
  });

  it("can be kept where something else keeps them", async () => {
    let theme = "auto";
    const store = localFieldStore([{ key: "theme", type: "select", options: ["auto", "dark"], default: "auto", get: () => theme, set: (v) => (theme = v as string) }], { prefix: "p." });
    await store.save({ theme: "dark" });
    expect(theme).toBe("dark");
    await store.save({ theme: null });
    expect(theme).toBe("auto");
  });
});

describe("the form", () => {
  const every: FieldState[] = [
    { key: "a.toggle", type: "toggle", label: "x.toggle", effective: false },
    { key: "a.select", type: "select", options: ["one", "two"], effective: "one" },
    { key: "a.number", type: "number", min: 1, max: 10, effective: 3 },
    { key: "a.text", type: "text", effective: "hello" },
    { key: "a.secret", type: "secret", effective: "***abcd" },
    { key: "a.list", type: "list", effective: ["x", "y"] },
    { key: "a.rules", type: "rules", columns: [{ key: "tool", required: true }, { key: "action", options: ["allow", "deny"], required: true }], effective: [{ tool: "Bash", action: "allow" }] },
  ];

  it("draws a control for each type, and writes a change through the store with the revision it read", async () => {
    const m = memory({ fields: every, revision: "r1" });
    render(<FieldForm store={m.store} />);
    const toggle = (await screen.findByTestId("settings-a.toggle")) as HTMLInputElement;
    expect(toggle.type).toBe("checkbox");
    expect((screen.getByTestId("settings-a.select") as HTMLSelectElement).value).toBe("one");
    expect((screen.getByTestId("settings-a.number") as HTMLInputElement).value).toBe("3");
    expect((screen.getByTestId("settings-a.text") as HTMLInputElement).value).toBe("hello");
    const secret = screen.getByTestId("settings-a.secret") as HTMLInputElement;
    expect(secret.type).toBe("password");
    expect(secret.value).toBe("");
    expect(secret.placeholder).toContain("***abcd");
    expect((screen.getByTestId("settings-a.list") as HTMLTextAreaElement).value).toBe("x\ny");
    expect(screen.getByTestId("settings-a.rules").querySelectorAll("tbody tr")).toHaveLength(1);

    fireEvent.click(toggle);
    await waitFor(() => expect(m.saved).toEqual([{ changes: { "a.toggle": true }, revision: "r1" }]));

    // Left with nothing new in it, a text is not written; with something new, it is.
    const text = screen.getByTestId("settings-a.text");
    fireEvent.blur(text);
    fireEvent.change(text, { target: { value: "world" } });
    fireEvent.blur(text);
    await waitFor(() => expect(m.saved.at(-1)?.changes).toEqual({ "a.text": "world" }));
    expect(m.saved).toHaveLength(2);

    // A list: one a line.
    const list = await screen.findByTestId("settings-a.list");
    fireEvent.change(list, { target: { value: "x\n\nz \n" } });
    fireEvent.blur(list);
    await waitFor(() => expect(m.saved.at(-1)?.changes).toEqual({ "a.list": ["x", "z"] }));

    // Rules: a row added and filled, written together.
    const rules = await screen.findByTestId("settings-a.rules");
    fireEvent.click(rules.querySelector('[data-testid="settings-rule-add"]')!);
    const row = rules.querySelectorAll("tbody tr")[1]!;
    fireEvent.change(row.querySelector("input")!, { target: { value: "Read" } });
    fireEvent.click(rules.querySelector('[data-testid="settings-rules-apply"]')!);
    await waitFor(() =>
      expect(m.saved.at(-1)?.changes).toEqual({ "a.rules": [{ tool: "Bash", action: "allow" }, { action: "allow", tool: "Read" }] }),
    );

    // A secret is written when typed, and the input is empty again.
    const secretAgain = screen.getByTestId("settings-a.secret") as HTMLInputElement;
    fireEvent.change(secretAgain, { target: { value: "sk-new" } });
    fireEvent.blur(secretAgain);
    await waitFor(() => expect(m.saved.at(-1)?.changes).toEqual({ "a.secret": "sk-new" }));
  });

  it("says a value it does not accept at its row, and writes nothing", async () => {
    const m = memory({ fields: [every[2]!] });
    render(<FieldForm store={m.store} />);
    const input = await screen.findByTestId("settings-a.number");
    fireEvent.change(input, { target: { value: "50" } });
    fireEvent.blur(input);
    expect(await screen.findByText(/A whole number from 1 to 10/)).toBeTruthy();
    expect(m.saved).toEqual([]);
  });

  it("says each value the store refused, at its row", async () => {
    const m = memory({ fields: [every[3]!] }, (changes) => ({ code: -32602, message: "no", data: { errors: Object.keys(changes).map((key) => ({ key, message: `${key} is wrong` })) } }));
    render(<FieldForm store={m.store} />);
    const input = await screen.findByTestId("settings-a.text");
    fireEvent.change(input, { target: { value: "x" } });
    fireEvent.blur(input);
    expect(await screen.findByText(/a\.text is wrong/)).toBeTruthy();
  });

  it("shows where a tiered value comes from, clears this tier, and restores the default", async () => {
    const mode: FieldState = {
      key: "permission_mode",
      type: "select",
      options: ["default", "plan", "bypassPermissions"],
      default: "default",
      writable: WRITABLE,
      effective: "plan",
      source: "project",
      withheld: false,
      tiers: tiers("acceptEdits", null, "plan"),
    };
    const m = memory({ fields: [mode], revision: "r" });
    const { rerender } = render(<FieldForm store={m.store} tier="project" tierName={(id) => `T:${id}`} />);
    expect(await screen.findByText(/Set in this tier · in force from T:project/)).toBeTruthy();
    // Clear gives this tier's value back.
    fireEvent.click(screen.getByTestId("settings-permission_mode-clear"));
    await waitFor(() => expect(m.saved.at(-1)?.changes).toEqual({ permission_mode: null }));
    // A lower tier writes it: restoring the default writes the default here.
    fireEvent.click(screen.getByTestId("settings-permission_mode-default"));
    await waitFor(() => expect(m.saved.at(-1)?.changes).toEqual({ permission_mode: "default" }));
    // Nothing below the global tier: there, restoring the default gives the value up.
    rerender(<FieldForm store={m.store} tier="global" tierName={(id) => `T:${id}`} />);
    fireEvent.click(await screen.findByTestId("settings-permission_mode-default"));
    await waitFor(() => expect(m.saved.at(-1)?.changes).toEqual({ permission_mode: null }));
  });

  it("says a looser project value is withheld, and a tier the field may not be written in", async () => {
    const field: FieldState = {
      key: "trust",
      type: "toggle",
      writable: ["global"],
      effective: false,
      source: "project",
      withheld: true,
      tiers: tiers(false, null, true),
      invalid: { scene: "true or false" },
    };
    render(<FieldForm store={memory({ fields: [field] }).store} tier="project" />);
    expect(await screen.findByText(/not in force: an untrusted project can only tighten settings/)).toBeTruthy();
    expect(screen.getByText(/not set in this tier; set it in: global/)).toBeTruthy();
    expect(screen.getByText(/the value in scene is not accepted \(true or false\)/)).toBeTruthy();
    expect((screen.getByTestId("settings-trust") as HTMLInputElement).disabled).toBe(true);
    expect(screen.queryByTestId("settings-trust-clear")).toBeNull();
  });

  it("asks before writing a risky value", async () => {
    const mode: FieldState = { key: "mode", type: "select", options: ["default", "bypassPermissions"], risk: { values: ["bypassPermissions"], message: "settings.risk.bypassPermissions" }, effective: "default" };
    const m = memory({ fields: [mode] });
    render(<FieldForm store={m.store} />);
    fireEvent.change(await screen.findByTestId("settings-mode"), { target: { value: "bypassPermissions" } });
    expect(await screen.findByTestId("settings-risk")).toBeTruthy();
    expect(m.saved).toEqual([]);
    const confirm = screen.getByTestId("settings-risk-confirm") as HTMLButtonElement;
    expect(confirm.disabled).toBe(true);
    fireEvent.click(screen.getByTestId("settings-risk-ack"));
    fireEvent.click(confirm);
    await waitFor(() => expect(m.saved).toEqual([{ changes: { mode: "bypassPermissions" }, revision: undefined }]));
    // Cancelled, nothing is written.
    fireEvent.change(screen.getByTestId("settings-mode"), { target: { value: "bypassPermissions" } });
    fireEvent.click(await screen.findByTestId("settings-risk-cancel"));
    expect(screen.queryByTestId("settings-risk")).toBeNull();
    expect(m.saved).toHaveLength(1);
  });

  it("keeps a change refused over settings that changed, shows it over them, and writes it at the new revision", async () => {
    const field = (value: number, other: string): FieldDoc => ({
      fields: [
        { key: "n", type: "number", min: 1, max: 100, effective: value },
        { key: "s", type: "text", effective: other },
      ],
      revision: `r-${value}-${other}`,
    });
    const m = memory(field(5, "a"), (_, revision) => (revision === "r-5-a" ? { code: CONFLICT, message: "moved" } : null));
    render(<FieldForm store={m.store} />);
    const input = await screen.findByTestId("settings-n");
    // Someone else wrote `s` meanwhile.
    m.set(field(5, "b"));
    fireEvent.change(input, { target: { value: "7" } });
    fireEvent.blur(input);
    expect(await screen.findByTestId("settings-conflict")).toBeTruthy();
    expect(m.saved).toEqual([]);

    fireEvent.click(screen.getByTestId("settings-conflict-keep"));
    expect(await screen.findByTestId("settings-review")).toBeTruthy();
    // Mine over theirs: the field shows the kept value, and says what is there now.
    expect((screen.getByTestId("settings-n") as HTMLInputElement).value).toBe("7");
    expect(screen.getByText(/your change, not saved yet \(now 5\)/)).toBeTruthy();
    expect(screen.getByText(/changed somewhere else/)).toBeTruthy();
    expect((screen.getByTestId("settings-s") as HTMLInputElement).value).toBe("b");

    fireEvent.click(screen.getByTestId("settings-review-save"));
    await waitFor(() => expect(m.saved).toEqual([{ changes: { n: 7 }, revision: "r-5-b" }]));
    await waitFor(() => expect(screen.queryByTestId("settings-review")).toBeNull());
  });

  it("draws only its section's fields", async () => {
    const doc = { fields: [{ ...every[0]!, section: "engine" }, { ...every[3]!, section: "models" }] };
    render(<FieldForm store={memory(doc).store} section="models" />);
    expect(await screen.findByTestId("settings-a.text")).toBeTruthy();
    expect(screen.queryByTestId("settings-a.toggle")).toBeNull();
  });
});

describe("a settings section that declares fields", () => {
  it("is drawn from them, and one with fields that are not declarations is refused", () => {
    const r = new Registry();
    expect(r.register("settings.section", { id: "prefs", label: () => "Prefs", fields: [{ key: "size", type: "number", default: 14 }] }, "ext")).toBe(true);
    const section = r.at("settings.section")[0]!;
    expect(typeof section.render).toBe("function");
    expect(section.fields?.[0]?.key).toBe("size");
    expect(r.register("settings.section", { id: "bad", label: () => "Bad", fields: [{ key: "x", type: "colour" } as never] }, "ext")).toBe(false);
    expect(r.register("settings.section", { id: "none", label: () => "None", fields: [] }, "ext")).toBe(false);
    expect(r.refusals().map((x) => x.id)).toEqual(["bad", "none"]);
  });

  it("keeps its values in this browser, under its owner", async () => {
    localStorage.clear();
    const r = new Registry();
    r.register("settings.section", { id: "prefs", label: () => "Prefs", fields: [{ key: "wrap", type: "toggle", default: false }] }, "ext");
    render(<>{r.at("settings.section")[0]!.render()}</>);
    fireEvent.click(await screen.findByTestId("settings-wrap"));
    await waitFor(() => expect(localStorage.getItem("harnix.pref.ext.wrap")).toBe("true"));
  });
});
