/**
 * `@harnix/ui-kit`: the shell's look in an application's isolated frame.
 *
 * Its stylesheet (`kit.css`) is the shell's design tokens — the same file the shell's theme is —
 * and a handful of components drawn with them (`hk-*` classes). This module follows the shell
 * through the bridge (`follow(app)`: the theme, the token values in force, the language) and
 * builds the components as plain DOM, so a page needs no framework to look native:
 *
 * ```js
 * const app = await HarnixApp.connect();
 * HarnixKit.follow(app);
 * document.body.append(
 *   HarnixKit.card({ title: "Notes", body: HarnixKit.empty("Nothing yet") }),
 *   HarnixKit.button("Add", { variant: "primary", onClick: add }),
 * );
 * ```
 *
 * A React page uses the classes alone (`className="hk-btn primary"`) and `follow`.
 */

import type { Environment } from "@harnix/app-sdk/protocol";

/** What `follow` needs of a connected app (`HarnixApp` is one). */
export interface Follows {
  readonly env: Environment;
  onEnv(fn: (env: Environment) => void): () => void;
}

/**
 * Set the shell's environment on a document: `data-theme`, `lang`, `data-narrow`, and every token
 * value as a custom property (so a brand's colours, which only the shell knows, are the frame's
 * too). Through the CSSOM: nothing inline is written into the document.
 */
export function applyEnv(env: Environment, root: HTMLElement = document.documentElement): void {
  root.dataset.theme = env.theme;
  root.lang = env.locale;
  if (env.narrow) root.dataset.narrow = "";
  else delete root.dataset.narrow;
  for (const [name, value] of Object.entries(env.tokens)) {
    if (name.startsWith("--") && typeof value === "string") root.style.setProperty(name, value);
  }
}

/** Apply the environment now and whenever it changes; returns the way to stop. */
export function follow(app: Follows, root?: HTMLElement): () => void {
  applyEnv(app.env, root);
  return app.onEnv((env) => applyEnv(env, root));
}

/* ── building blocks ──────────────────────────────────────────────────── */

type Child = Node | string | number | null | undefined | false;

export type Props = {
  className?: string;
  /** `data-testid`, which tests find a component by. */
  testid?: string;
  [attribute: string]: unknown;
};

/** An element: `on<Event>` props are listeners, the rest attributes (`true` is present, false and
 *  null absent); children are nodes or text. */
export function h<K extends keyof HTMLElementTagNameMap>(tag: K, props: Props = {}, ...children: Child[]): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [name, value] of Object.entries(props)) {
    if (value === undefined || value === null || value === false) continue;
    if (name === "className") el.className = String(value);
    else if (name === "testid") el.dataset.testid = String(value);
    else if (/^on[A-Z]/.test(name) && typeof value === "function") el.addEventListener(name.slice(2).toLowerCase(), value as EventListener);
    else if (name === "value" && "value" in el) (el as HTMLInputElement).value = String(value);
    else el.setAttribute(name, value === true ? "" : String(value));
  }
  el.append(...children.filter((c): c is Node | string | number => c !== null && c !== undefined && c !== false).map((c) => (typeof c === "number" ? String(c) : c)));
  return el;
}

const classes = (...names: (string | false | undefined)[]) => names.filter(Boolean).join(" ");

export function button(
  label: Child,
  options: { onClick?: (event: MouseEvent) => void; variant?: "primary" | "ghost" | "danger"; size?: "sm"; disabled?: boolean; testid?: string; type?: "button" | "submit"; title?: string } = {},
): HTMLButtonElement {
  return h(
    "button",
    {
      type: options.type ?? "button",
      className: classes("hk-btn", options.variant, options.size),
      onClick: options.onClick,
      disabled: options.disabled,
      testid: options.testid,
      title: options.title,
    },
    label,
  );
}

export function input(
  options: { value?: string; placeholder?: string; type?: string; name?: string; multiline?: boolean; onInput?: (value: string, event: Event) => void; testid?: string } = {},
): HTMLInputElement | HTMLTextAreaElement {
  const el = h(options.multiline ? "textarea" : "input", {
    className: "hk-input",
    type: options.multiline ? undefined : (options.type ?? "text"),
    name: options.name,
    placeholder: options.placeholder,
    value: options.value,
    testid: options.testid,
  });
  if (options.onInput) el.addEventListener("input", (event) => options.onInput!((el as HTMLInputElement).value, event));
  return el;
}

/** A labelled control. */
export function field(label: Child, control: Node): HTMLLabelElement {
  return h("label", { className: "hk-field" }, h("span", { className: "hk-label" }, label), control);
}

export function card(options: { title?: Child; body?: Child | Child[]; testid?: string } = {}): HTMLElement {
  const body = Array.isArray(options.body) ? options.body : [options.body];
  return h("section", { className: "hk-card", testid: options.testid }, options.title ? h("div", { className: "hk-card-title" }, options.title) : null, ...body);
}

export interface Column<Row> {
  label: Child;
  /** The cell: a field of the row, or what to draw from it. */
  cell: keyof Row | ((row: Row) => Child);
}

export function table<Row extends object>(columns: Column<Row>[], rows: Row[], options: { testid?: string; empty?: Child; onRow?: (row: Row) => void } = {}): HTMLElement {
  if (!rows.length && options.empty !== undefined) return empty(options.empty);
  const head = h("thead", {}, h("tr", {}, ...columns.map((c) => h("th", {}, c.label))));
  const body = h(
    "tbody",
    {},
    ...rows.map((row) =>
      h(
        "tr",
        { onClick: options.onRow ? () => options.onRow!(row) : undefined },
        ...columns.map((c) => h("td", {}, typeof c.cell === "function" ? c.cell(row) : (String((row as Record<PropertyKey, unknown>)[c.cell as PropertyKey] ?? "")))),
      ),
    ),
  );
  return h("table", { className: "hk-table", testid: options.testid }, head, body);
}

export function empty(text: Child, testid?: string): HTMLElement {
  return h("div", { className: "hk-empty", testid }, text);
}

export function loading(text?: Child, testid?: string): HTMLElement {
  return h("div", { className: "hk-loading", role: "status", testid }, text ?? "");
}

export function alert(text: Child, options: { tone?: "error" | "info"; testid?: string } = {}): HTMLElement {
  return h("div", { className: classes("hk-alert", options.tone === "info" && "info"), role: options.tone === "info" ? "status" : "alert", testid: options.testid }, text);
}

/** Replace what is in `parent` with `children`. */
export function render(parent: Element, ...children: Child[]): void {
  parent.replaceChildren(...children.filter((c): c is Node | string | number => c !== null && c !== undefined && c !== false).map((c) => (typeof c === "number" ? String(c) : c)));
}
