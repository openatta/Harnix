/** Settings drawn from their declarations: what each type accepts, this browser's store, and the form. */

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { CONFLICT } from "@harnix/core";

import { FieldForm, Registry, checkField, isRisky, localFieldStore, routeOf, setLocale, toolPattern, type FieldDoc, type FieldState, type FieldStore } from "../src/index";

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

  it("shows a json value and accepts none of it as a write", async () => {
    expect(checkField({ key: "execution", type: "json" }, { max_agent_depth: 3 })).toMatchObject({ key: "settings.invalid.readonly" });
    const field = { key: "execution", type: "json", section: "advanced", writable: [], effective: { max_agent_depth: 3 }, source: "global", tiers: { global: { max_agent_depth: 3 }, scene: null, project: null } } as FieldState;
    const { store } = memory({ fields: [field] });
    render(<FieldForm store={store} tier="global" />);
    expect((await screen.findByTestId("settings-execution")).textContent).toBe('{"max_agent_depth":3}');
    expect(document.querySelector("input, select")).toBeNull();
  });

  it("takes a permission rule's tool only in a shape the engine can match", () => {
    for (const good of ["Bash", "Bash(git push:*)", "Edit(src/**)", "WebFetch(https://*.example.com/**)", "mcp__github", "plugin__github-tools__open_pr", "Bash(echo (hi))"]) {
      expect(toolPattern(good), good).toBeNull();
    }
    expect(toolPattern("Bash(git")).toBe("settings.invalid.toolParen");
    expect(toolPattern("Bash)")).toBe("settings.invalid.toolParen");
    expect(toolPattern("mcp__github__*")).toBe("settings.invalid.toolWildcard");
    expect(toolPattern("(git status)")).toBe("settings.invalid.toolName");
    expect(toolPattern("mcp__")).toBe("settings.invalid.toolName");
    expect(toolPattern("Bash Tool")).toBe("settings.invalid.toolName");
    expect(toolPattern("Bash()")).toBe("settings.invalid.toolEmpty");
    const rules = {
      key: "permission_rules",
      type: "rules" as const,
      columns: [
        { key: "tool", required: true, check: "tool" as const },
        { key: "action", options: ["allow", "ask", "deny"], required: true },
        { key: "match", options: ["pattern", "exact"] },
      ],
    };
    expect(checkField(rules, [{ tool: "Bash(git push:*)", action: "deny" }, { tool: "Read", action: "allow", match: "exact" }])).toBeNull();
    expect(checkField(rules, [{ tool: "Bash", action: "allow" }, { tool: "mcp__x__*", action: "deny" }])).toEqual({ key: "settings.invalid.toolWildcard", vars: { row: 2 } });
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

  it("edits a permission rule as its tool and what the parentheses hold, and writes the engine's shape", async () => {
    const rules: FieldState = {
      key: "permission_rules",
      type: "rules",
      columns: [
        { key: "tool", required: true, check: "tool", label: "settings.column.permission_rules.tool" },
        { key: "action", type: "select", options: ["allow", "ask", "deny"], required: true, label: "settings.column.permission_rules.action" },
        { key: "match", type: "select", options: ["pattern", "exact"] },
      ],
      effective: [{ tool: "Bash(git push:*)", action: "deny" }],
    };
    const m = memory({ fields: [rules] });
    render(<FieldForm store={m.store} />);
    const table = await screen.findByTestId("settings-permission_rules");
    expect(table.textContent).toContain("Effect");
    expect((table.querySelector('[data-testid="settings-rule-tool"]') as HTMLInputElement).value).toBe("Bash");
    expect((table.querySelector('[data-testid="settings-rule-content"]') as HTMLInputElement).value).toBe("git push:*");

    fireEvent.click(table.querySelector('[data-testid="settings-rule-add"]')!);
    const row = table.querySelectorAll("tbody tr")[1]!;
    fireEvent.change(row.querySelector('[data-testid="settings-rule-tool"]')!, { target: { value: "Edit" } });
    // A space typed at the end stays while typing; what is written is trimmed.
    fireEvent.change(row.querySelector('[data-testid="settings-rule-content"]')!, { target: { value: "src/** " } });
    expect((row.querySelector('[data-testid="settings-rule-content"]') as HTMLInputElement).value).toBe("src/** ");
    fireEvent.click(table.querySelector('[data-testid="settings-rules-apply"]')!);
    await waitFor(() =>
      expect(m.saved.at(-1)?.changes).toEqual({ permission_rules: [{ tool: "Bash(git push:*)", action: "deny" }, { action: "allow", tool: "Edit(src/**)" }] }),
    );

    // A wildcard in the tool's name is said at the row, and nothing is written.
    const before = m.saved.length;
    const table2 = await screen.findByTestId("settings-permission_rules");
    fireEvent.change(table2.querySelector('[data-testid="settings-rule-tool"]')!, { target: { value: "mcp__github__*" } });
    fireEvent.click(table2.querySelector('[data-testid="settings-rules-apply"]')!);
    expect(await screen.findByText(/Row 1: a tool's name has no wildcard/)).toBeTruthy();
    expect(m.saved).toHaveLength(before);
  });

  it("edits a tier's own rules of an additive field, and says how many the other tiers add", async () => {
    const deny = { tool: "Bash(rm:*)", action: "deny" };
    const allow = { tool: "Bash(ls)", action: "allow" };
    const rules: FieldState = {
      key: "permission_rules",
      type: "rules",
      additive: true,
      columns: [
        { key: "tool", required: true, check: "tool" },
        { key: "action", options: ["allow", "ask", "deny"], required: true },
      ],
      writable: WRITABLE,
      effective: [deny, allow],
      source: "project",
      withheld: false,
      tiers: tiers([deny], null, [allow]),
    };
    const m = memory({ fields: [rules], revision: "r" });
    render(<FieldForm store={m.store} tier="global" />);
    const table = await screen.findByTestId("settings-permission_rules");
    expect(table.querySelectorAll("tbody tr")).toHaveLength(1);
    expect((table.querySelector('[data-testid="settings-rule-content"]') as HTMLInputElement).value).toBe("rm:*");
    expect(screen.getByText(/every tier's rules apply together; 1 more from the other tiers/)).toBeTruthy();
    fireEvent.click(table.querySelector('[data-testid="settings-rule-remove"]')!);
    fireEvent.click(table.querySelector('[data-testid="settings-rules-apply"]')!);
    // Only this tier's rows: the project's allow is not copied into the global tier.
    await waitFor(() => expect(m.saved.at(-1)?.changes).toEqual({ permission_rules: [] }));
  });

  it("names a select's options by their text keys", async () => {
    const mode: FieldState = {
      key: "permission_mode",
      type: "select",
      options: ["default", "bypassPermissions"],
      optionLabels: { default: "settings.option.permission_mode.default", bypassPermissions: "settings.option.permission_mode.bypassPermissions" },
      effective: "default",
    };
    render(<FieldForm store={memory({ fields: [mode] }).store} />);
    const select = (await screen.findByTestId("settings-permission_mode")) as HTMLSelectElement;
    expect([...select.options].map((o) => o.textContent)).toEqual(["Ask (default)", "Full access"]);
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

describe("a model route", () => {
  const route = (effective: unknown): FieldState => ({
    key: "task_models.title",
    type: "route",
    effective,
    routes: [
      { provider: "deepseek", default_model: "deepseek-chat", models: ["deepseek-chat", "deepseek-reasoner"] },
      { provider: "anthropic", default_model: null, models: [] },
    ],
  });

  it("is a provider, or a provider and one model of it", () => {
    const field = { key: "r", type: "route" as const };
    expect(checkField(field, "deepseek")).toBeNull();
    expect(checkField(field, { provider: "deepseek", model: "deepseek-chat" })).toBeNull();
    expect(checkField(field, { provider: "deepseek", model: null })).toBeNull();
    expect(checkField(field, { model: "m" })).toMatchObject({ key: "settings.invalid.route" });
    expect(checkField(field, { provider: "a", model: " " })).not.toBeNull();
    expect(checkField(field, { provider: "a", base_url: "http://x" })).not.toBeNull();
    expect(checkField(field, "")).not.toBeNull();
    expect(routeOf("a")).toEqual({ provider: "a", model: null });
    expect(routeOf({ provider: "a", model: "m" })).toEqual({ provider: "a", model: "m" });
    expect(routeOf(null)).toEqual({ provider: null, model: null });
  });

  it("is picked from the providers configured, its model typed or offered, blank for the default", async () => {
    const m = memory({ fields: [route(null)], revision: "r1" });
    render(<FieldForm store={m.store} />);
    const provider = (await screen.findByTestId("settings-task_models.title")) as HTMLSelectElement;
    expect([...provider.options].map((o) => [o.value, o.disabled])).toEqual([["", true], ["deepseek", false], ["anthropic", false]]);
    // No provider yet: no model to name.
    expect((screen.getByTestId("settings-task_models.title-model") as HTMLInputElement).disabled).toBe(true);
    fireEvent.change(provider, { target: { value: "deepseek" } });
    await waitFor(() => expect(m.saved).toEqual([{ changes: { "task_models.title": "deepseek" }, revision: "r1" }]));
  });

  it("writes a model as {provider, model}, and offers the provider's models", async () => {
    const m = memory({ fields: [route("deepseek")], revision: "r1" });
    render(<FieldForm store={m.store} />);
    const model = (await screen.findByTestId("settings-task_models.title-model")) as HTMLInputElement;
    expect(model.placeholder).toBe("deepseek-chat");
    const offered = document.getElementById(model.getAttribute("list")!)!;
    expect([...offered.querySelectorAll("option")].map((o) => o.getAttribute("value"))).toEqual(["deepseek-chat", "deepseek-reasoner"]);
    fireEvent.change(model, { target: { value: "deepseek-reasoner" } });
    fireEvent.blur(model);
    await waitFor(() => expect(m.saved).toEqual([{ changes: { "task_models.title": { provider: "deepseek", model: "deepseek-reasoner" } }, revision: "r1" }]));
  });

  it("goes back to the provider alone when its model is cleared", async () => {
    const m = memory({ fields: [route({ provider: "deepseek", model: "deepseek-chat" })], revision: "r1" });
    render(<FieldForm store={m.store} />);
    const model = (await screen.findByTestId("settings-task_models.title-model")) as HTMLInputElement;
    expect(model.value).toBe("deepseek-chat");
    fireEvent.change(model, { target: { value: "" } });
    fireEvent.blur(model);
    await waitFor(() => expect(m.saved).toEqual([{ changes: { "task_models.title": "deepseek" }, revision: "r1" }]));
  });

  it("says so when no provider is configured", async () => {
    const m = memory({ fields: [{ ...route(null), routes: [] }] });
    render(<FieldForm store={m.store} />);
    expect(((await screen.findByTestId("settings-task_models.title")) as HTMLSelectElement).disabled).toBe(true);
    expect(screen.getByText(/No provider configured yet/)).toBeTruthy();
  });

  it("a text field offers the models it is given, and takes one it is not", async () => {
    const m = memory({ fields: [{ key: "model.fallback_model", type: "text", effective: null, suggestions: ["a-1", "b-1"] }] });
    render(<FieldForm store={m.store} />);
    const input = (await screen.findByTestId("settings-model.fallback_model")) as HTMLInputElement;
    const offered = document.getElementById(input.getAttribute("list")!)!;
    expect([...offered.querySelectorAll("option")].map((o) => o.getAttribute("value"))).toEqual(["a-1", "b-1"]);
    fireEvent.change(input, { target: { value: "not-offered" } });
    fireEvent.blur(input);
    await waitFor(() => expect(m.saved).toEqual([{ changes: { "model.fallback_model": "not-offered" }, revision: undefined }]));
  });
});
