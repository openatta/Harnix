/**
 * 贡献点注册表（概念 §5.1、设计 §10.3）：界面上哪些地方可以由内建或运行期包填，以及怎么填。
 *
 * v2 的八个点，都注册 React 组件：
 *
 * | 点 | 形状 | 怎么用 |
 * |---|---|---|
 * | `space` | `id`、`icon`、`title()`、`place?`、`details?`、`center`、`sidebar?()`、`overview?()`、`tabs?` | 全部，按 `order` 再按注册顺序 |
 * | `home.card` | `id`、`render()`、`title?()` | 全部，按 `order` 再按注册顺序 |
 * | `tool.row` | `match(tool块)`、`render(tool块)` | 后注册的先认领 |
 * | `details.panel` | `match(item)`、`render(item)`、`title?(item)` | 后注册的先认领 |
 * | `flow.block` | `match(块)`、`render(块)` | 后注册的先认领 |
 * | `sidebar.group` | `group(会话) → 组[]`、`label?()` | 只用最后注册的那一个 |
 * | `command` | `trigger?`（`/` 或 `@`）、`complete(query, ctx?) → 候选[]`（候选可带 `run`） | 全部，按注册顺序拼起来；`/` 与 `@` 两个菜单各问各的 |
 * | `settings.section` | `id`、`label()`、`render()` | 全部，按注册顺序 |
 *
 * 不是贡献点、但同样按 owner 记的还有三样：令牌覆盖（`tokens`）、语言包（`strings`）与图标（`icons`）。
 *
 * 注册不合形状的贡献被拒绝并记下理由；`match` 或 `render` 抛错只算这一个贡献失败，点照常工作。
 * 被拒的、失败的都列在诊断页里。
 */

import { createElement, Component, useSyncExternalStore, type ErrorInfo, type ReactNode } from "react";
import type { Block, BlockOf } from "@harnix/core/state";
import type { SessionRow } from "@harnix/core";
import type { ChatIds } from "./chat/types";
import type { SessionRef } from "./host";
import { addStrings, type Locale } from "./i18n/index";

export const POINTS = [
  "space",
  "home.card",
  "tool.row",
  "details.panel",
  "flow.block",
  "sidebar.group",
  "command",
  "settings.section",
] as const;
export type Point = (typeof POINTS)[number];

/** 右栏详情里打开的东西：一行工具调用、一次请求信封，或贡献自己的。 */
export interface DetailItem {
  kind: string;
  name?: string;
  [key: string]: unknown;
}

export interface SidebarGroup {
  key: string;
  title: string;
  hint?: string;
  /** 这一组对应的工作区（可以折叠、拖动、改名） */
  workspace?: { id: string; path: string; title?: string | null; collapsed?: boolean };
  rows: SessionRow[];
}

export interface CommandCandidate {
  replacement: string;
  label: string;
  description?: string;
  source?: string;
  /** Given: choosing it runs this instead — nothing is inserted or sent, and the `/xxx` typed so
   *  far is cleared. A failure is shown as the shell's error. */
  run?(ctx: CommandContext): void | Promise<void>;
}

/** Where a command or mention is being asked for: the front space and its open session. */
export interface CommandContext {
  space: string;
  session: SessionRef | null;
}

export interface SpaceTab {
  id: string;
  title(): string;
  render(): ReactNode;
  /** `end`: drawn after the others, at the far end of the strip — for tabs used less often. */
  align?: "start" | "end";
}

/** A space's right-column tabs, whichever way it gave them. */
export function tabsOf(space: SpaceContribution | undefined): SpaceTab[] {
  const tabs = space?.tabs;
  return typeof tabs === "function" ? tabs() : (tabs ?? []);
}

/** A space as the interface draws it (design §8.1). Where its sessions land is the backend's. */
export interface SpaceContribution {
  id: string;
  /** A glyph name. */
  icon: string;
  title(): string;
  order?: number;
  /** Where the left column lists it: with the product's spaces (`top`, the default) or at the
   *  bottom, above the system entry — for what is global rather than one of the surfaces. */
  place?: "top" | "bottom";
  /** The right column when this space is first entered; afterwards each space keeps its own. */
  details?: "open" | "closed";
  /** What the centre shows: the conversation, the overview with the conversation as a bar under
   *  it, or the overview alone. */
  center?: "chat" | "minimized" | "hidden";
  /** The left column below the space list: its entries. The space's own sessions when absent. */
  sidebar?(): ReactNode;
  /** The centre when it is not the conversation. */
  overview?(): ReactNode;
  /** Right-column tabs, beside the details drawer. A function when they depend on what is
   *  selected, read at every draw. */
  tabs?: SpaceTab[] | (() => SpaceTab[]);
  /** The backend space whose sessions this one lists and creates; its own id when absent. Two
   *  views of the same sessions — the session space by time, the projects space by project —
   *  are two interface spaces over one backend space. */
  sessions?: string;
  /** A line under the title: what this space is for. */
  subtitle?(): string;
  /** What a new conversation here might start with, offered under the composer. */
  suggestions?(): string[];
  /** One line under the composer (what the Agent may and may not do here). */
  notice?(): string;
  placeholder?(): string;
  /** A badge on the space's entry in the left column: a number or a very short text. Nothing is
   *  drawn for null, undefined, 0 or "". Read at every draw; call `host.redraw()` when it changes. */
  badge?(): string | number | null | undefined;
  /** The open session is read-only: the words that take the composer's place. False or undefined:
   *  it can be written to. Only the interface — the hub still takes `harnix.send`; this guards
   *  against mistakes, it is not a security boundary. */
  readOnly?(session: SessionRef): string | false | undefined;
  /** What stays at the top of the open session's flow, above the messages; null or undefined for
   *  nothing. Read at every draw of the conversation. */
  pinned?(session: SessionRef): ReactNode;
  /** A line over the composer of an open session (`notice()` is the new conversation's); null or
   *  undefined for none. Read at every draw of the conversation. */
  composerNotice?(session: SessionRef): ReactNode;
  /** Which words in the conversation's text are ids that open something, and what opening one
   *  does. The pattern carries the `g` flag (the shell adds it when missing, and notes that). */
  ids?(): ChatIds | undefined;
  /** The composer's options. `steer`: a "deliver now" switch under it — while a turn runs, a
   *  message goes into that turn at its next step instead of waiting for it to end. */
  composer?: { steer?: boolean };
  /** Command names (without the `/`) left out of this space's `/` menu — other owners' commands
   *  only, so the space's own command of the same name takes the place of the engine's. Typed by
   *  hand, such a command is still sent as text: this guards against mistakes, it forbids nothing. */
  hideCommands?: string[];
}

export interface Contributions {
  space: SpaceContribution;
  "home.card": { id: string; render(): ReactNode; title?(): string; order?: number };
  "tool.row": { id: string; match(block: BlockOf<"tool">): boolean; render(block: BlockOf<"tool">): ReactNode };
  "details.panel": { id: string; match(item: DetailItem): boolean; render(item: DetailItem): ReactNode; title?(item: DetailItem): string };
  "flow.block": { id: string; match(block: Block): boolean; render(block: Block): ReactNode };
  "sidebar.group": { id: string; group(sessions: SessionRow[]): SidebarGroup[]; label?(): string };
  command: {
    id: string;
    /** Which leading character's menu asks it: `/` (the default) or `@`. */
    trigger?: "/" | "@";
    complete(query: string, ctx?: CommandContext): CommandCandidate[] | Promise<CommandCandidate[]>;
  };
  "settings.section": { id: string; label(): string; render(): ReactNode };
}

const SHAPE: { [P in Point]: string[] } = {
  space: ["title"],
  "home.card": ["render"],
  "tool.row": ["match", "render"],
  "details.panel": ["match", "render"],
  "flow.block": ["match", "render"],
  "sidebar.group": ["group"],
  command: ["complete"],
  "settings.section": ["label", "render"],
};

export interface Refusal {
  point: string;
  owner: string;
  id?: string;
  reason: string;
}

type Entry<P extends Point> = Contributions[P] & { owner: string };

export class Registry {
  private readonly entries = new Map<Point, Entry<Point>[]>(POINTS.map((p) => [p, []]));
  private readonly overrides = new Map<string, { value: string; owner: string }>();
  private readonly glyphs = new Map<string, string>();
  private readonly refused: Refusal[] = [];
  /** The token names a module may override: the ones the page's stylesheets declare on `:root`.
   *  Set by the shell once its styles are in; until then every override waits. */
  private known: Set<string> | null = null;
  private pending: { tokens: Record<string, string>; owner: string }[] = [];
  private readonly listeners = new Set<() => void>();
  private version = 0;

  register<P extends Point>(point: P, contribution: Contributions[P], owner: string): boolean;
  register(point: string, contribution: unknown, owner: string): boolean;
  register(point: string, contribution: unknown, owner: string): boolean {
    const list = this.entries.get(point as Point);
    const c = contribution as Record<string, unknown> | null;
    if (!list) return this.refuse({ point, owner, reason: `there is no contribution point \`${point}\`` });
    if (!c || typeof c !== "object" || typeof c.id !== "string") {
      return this.refuse({ point, owner, reason: "a contribution needs an `id`" });
    }
    const missing = SHAPE[point as Point].filter((name) => typeof c[name] !== "function");
    if (missing.length) {
      return this.refuse({ point, owner, id: c.id, reason: `missing ${missing.join(", ")}` });
    }
    if (point === "command" && c.trigger !== undefined && c.trigger !== "/" && c.trigger !== "@") {
      return this.refuse({ point, owner, id: c.id, reason: `a command's trigger is "/" or "@", not ${JSON.stringify(c.trigger)}` });
    }
    list.push({ ...(c as object), owner } as Entry<Point>);
    this.changed();
    return true;
  }

  /**
   * Token overrides (design §10.5): a module's colours for names the theme already has. A name the
   * theme does not declare is refused with the reason — a module adds no colours of its own — and
   * so is a value that reaches outside the page (`url(`). Not a contribution point: it draws
   * nothing. A later override of the same name wins, as a later registration does.
   */
  tokens(overrides: Record<string, string>, owner: string): void {
    if (!this.known) {
      this.pending.push({ tokens: overrides, owner });
      return;
    }
    for (const [name, value] of Object.entries(overrides)) {
      if (!this.known.has(name)) {
        this.refuse({ point: "tokens", owner, id: name, reason: `\`${name}\` is not a token of this theme` });
      } else if (typeof value !== "string" || /url\(|expression|[;{}]/i.test(value)) {
        this.refuse({ point: "tokens", owner, id: name, reason: "a token's value is a colour or a length, nothing more" });
      } else {
        this.overrides.set(name, { value, owner });
      }
    }
    this.changed();
  }

  /** The theme's token names, once the page knows them; overrides waiting for them are applied. */
  knowTokens(names: Iterable<string>): void {
    this.known = new Set(names);
    const waiting = this.pending;
    this.pending = [];
    for (const { tokens, owner } of waiting) this.tokens(tokens, owner);
    this.changed();
  }

  /** The overrides in force, by token name. */
  tokenOverrides(): Record<string, string> {
    return Object.fromEntries([...this.overrides].map(([name, o]) => [name, o.value]));
  }

  /** A module's language pack; keys outside `<owner>.` are refused (design §10.5). */
  strings(locale: Locale, dictionary: Record<string, string>, owner: string): void {
    for (const key of addStrings(locale, dictionary, owner)) {
      this.refuse({ point: "strings", owner, id: key, reason: `a key of ${owner}'s pack is \`${owner}.…\`` });
    }
    this.changed();
  }

  /**
   * An application's own icons: name → the inside of an SVG (no `<svg>`), drawn on the shell's
   * 24 grid at its 1.7 stroke, in `currentColor`. Each is `<owner>.<name>`, which is what a space's
   * `icon` names; a name without a dot is still the shell's. Only shapes and their geometry and
   * stroke are taken — a script, an `on*` handler, a link, a style, a reference to anything else
   * refuses the whole icon, with the reason. A guard against mistakes: interface modules are
   * trusted as the page is. Not a contribution point: it draws nothing by itself.
   */
  icons(icons: Record<string, string>, owner: string): void {
    for (const [name, body] of Object.entries(icons)) {
      const full = `${owner}.${name}`;
      if (!/^[A-Za-z0-9_-]+$/.test(name)) {
        this.refuse({ point: "icons", owner, id: full, reason: "an icon's name is letters, digits, - and _" });
        continue;
      }
      const clean = cleanSvg(typeof body === "string" ? body : "");
      if (typeof clean !== "string") {
        this.refuse({ point: "icons", owner, id: full, reason: clean.refused });
        continue;
      }
      this.glyphs.set(full, clean);
    }
    this.changed();
  }

  /** An application icon's inside, as `icons` took it; undefined for a name it does not have. */
  icon(name: string): string | undefined {
    return this.glyphs.get(name);
  }

  /** Every registration of a point, in order. */
  at<P extends Point>(point: P): Entry<P>[] {
    return (this.entries.get(point) ?? []) as unknown as Entry<P>[];
  }

  /** The newest registration that claims `subject`. */
  claim<P extends "tool.row" | "details.panel" | "flow.block">(point: P, subject: Parameters<Contributions[P]["match"]>[0]): Entry<P> | null {
    const list = this.at(point);
    for (let i = list.length - 1; i >= 0; i -= 1) {
      const c = list[i]!;
      try {
        if ((c.match as (s: unknown) => boolean)(subject)) return c;
      } catch (e) {
        this.fail(point, c, e);
      }
    }
    return null;
  }

  /** The last `sidebar.group` registration: grouping is one decision, not a merge. */
  grouping(): Entry<"sidebar.group"> | null {
    const list = this.at("sidebar.group");
    return list.length ? list[list.length - 1]! : null;
  }

  refusals(): Refusal[] {
    return [...this.refused];
  }

  /** Record a contribution that broke while being used. */
  fail(point: string, c: { id: string; owner: string }, error: unknown): void {
    this.refuse({ point, owner: c.owner, id: c.id, reason: error instanceof Error ? error.message : String(error) });
  }

  /** A package or module the shell could not use at all. */
  note(refusal: Refusal): void {
    this.refuse(refusal);
  }

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  readonly getVersion = (): number => this.version;

  private refuse(refusal: Refusal): false {
    // Only something new is a change: a contribution failing at every draw would otherwise draw again.
    if (!this.refused.some((r) => r.point === refusal.point && r.owner === refusal.owner && r.id === refusal.id && r.reason === refusal.reason)) {
      this.refused.push(refusal);
      this.changed();
    }
    return false;
  }

  private changed(): void {
    this.version += 1;
    for (const fn of [...this.listeners]) fn();
  }
}

const SHAPES = new Set(["path", "circle", "rect", "line", "polyline", "polygon", "ellipse", "g"]);
const GEOMETRY = new Set([
  "d", "cx", "cy", "r", "rx", "ry", "x", "y", "x1", "y1", "x2", "y2", "width", "height", "points", "transform", "pathLength",
  "fill", "fill-opacity", "fill-rule", "clip-rule", "opacity",
  "stroke", "stroke-width", "stroke-linecap", "stroke-linejoin", "stroke-dasharray", "stroke-dashoffset", "stroke-miterlimit", "stroke-opacity",
  "vector-effect",
]);

/** An SVG's inside, rebuilt from the shapes it may have; or why it may not. */
function cleanSvg(body: string): string | { refused: string } {
  const Parser = (globalThis as { DOMParser?: typeof DOMParser }).DOMParser;
  if (!Parser) return { refused: "icons need a page to be read in" };
  const doc = new Parser().parseFromString(`<svg xmlns="http://www.w3.org/2000/svg">${body}</svg>`, "image/svg+xml");
  if (doc.getElementsByTagName("parsererror").length) return { refused: "not well-formed SVG" };
  const escape = (v: string) => v.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
  const walk = (node: Element): string | { refused: string } => {
    const out: string[] = [];
    for (const child of Array.from(node.childNodes)) {
      if (child.nodeType === 3) {
        if ((child.textContent ?? "").trim()) return { refused: "an icon has no text in it" };
        continue;
      }
      if (child.nodeType === 8) continue;
      if (child.nodeType !== 1) return { refused: "only shapes go in an icon" };
      const el = child as Element;
      const tag = el.localName;
      if (!SHAPES.has(tag)) return { refused: `<${tag}> is not a shape an icon may have` };
      const attrs: string[] = [];
      for (const attr of Array.from(el.attributes)) {
        const name = attr.name;
        if (!GEOMETRY.has(name)) return { refused: `${name} is not an attribute an icon may have` };
        if (/url\(|javascript:|expression|[<>]/i.test(attr.value)) return { refused: `${name} may not refer to anything` };
        attrs.push(` ${name}="${escape(attr.value)}"`);
      }
      const inner = walk(el);
      if (typeof inner !== "string") return inner;
      out.push(inner ? `<${tag}${attrs.join("")}>${inner}</${tag}>` : `<${tag}${attrs.join("")}/>`);
    }
    return out.join("");
  };
  const result = walk(doc.documentElement);
  if (typeof result === "string" && !result) return { refused: "an icon needs at least one shape" };
  return result;
}

/** Re-render when contributions are added or refused. */
export function useRegistry(registry: Registry): Registry {
  useSyncExternalStore(registry.subscribe, registry.getVersion);
  return registry;
}

/**
 * Render a contribution, and if it throws, count it as failed rather than
 * taking the page down with it.
 */
export function Contributed({ registry, point, contribution, children }: {
  registry: Registry;
  point: string;
  contribution: { id: string; owner: string };
  children: ReactNode;
}) {
  return createElement(Guard, { registry, point, contribution, children });
}

interface GuardProps {
  registry: Registry;
  point: string;
  contribution: { id: string; owner: string };
  children: ReactNode;
}

class Guard extends Component<GuardProps, { failed: boolean }> {
  override state = { failed: false };

  static getDerivedStateFromError(): { failed: boolean } {
    return { failed: true };
  }

  override componentDidCatch(error: Error, _info: ErrorInfo): void {
    this.props.registry.fail(this.props.point, this.props.contribution, error);
  }

  override render(): ReactNode {
    return this.state.failed ? null : this.props.children;
  }
}
