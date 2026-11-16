/**
 * 贡献点注册表（概念 §5.6）：界面上哪些地方可以由内建、UI 包或运行期包填，以及怎么填。
 *
 * v4 的十二个内核点：
 *
 * | 点 | 形状 | 怎么用 |
 * |---|---|---|
 * | `space` | `id`、`icon`、`title()`，与四组 `sidebar`、`center`、`conversation`、`details`，可选的 `store` | 全部，按 `order` 再按注册顺序 |
 * | `home.card` | `id`、`render()`、`title?()` | 全部，按 `order` 再按注册顺序；没有 `title`、`render()` 什么都没画的卡不占位 |
 * | `tool.row` | `match(tool块)`、`render(tool块)` | 后注册的先认领 |
 * | `details.panel` | `match(item)`、`render(item)`、`title?(item)` | 后注册的先认领 |
 * | `flow.block` | `match(块)`、`render(块)` | 后注册的先认领 |
 * | `sidebar.group` | `group(会话) → 组[]`、`label?()` | 只用最后注册的那一个 |
 * | `command` | `trigger?`（`/` 或 `@`）、`complete(query, ctx?) → 候选[]`（候选可带 `run`） | 全部，按注册顺序拼起来；`/` 与 `@` 两个菜单各问各的 |
 * | `settings.section` | `id`、`label()`、`render()`，可选的 `covers`；或不给 `render`，给 `fields`（字段声明，见 `fields.tsx`）与可选的 `store` | 全部，按注册顺序；外壳要人去配模型时打开 `covers` 里有 `"models"` 的那一节；只给 `fields` 的按声明画表单，值缺省存在这个浏览器里 |
 * | `session.action` | `id`、`at`、`label(target)`、`match(target)`、`run(target)`，可选的 `disabled(target)` | 标题栏、会话菜单、工具行（按钮与参数）、人的消息的菜单（`message`）各问 `at` 里有它的，按注册顺序 |
 * | `sidebar.tab` | `id`、`kind`、`title(address?)`、`render()`，可选的 `patterns`、`priority`、`canOpen`、`guide`、`multiple`、`keepMounted`、`scope` | 右栏的页签类型（`sidebar.ts`）：资源地址按优先级档、模式长度、注册顺序认领；一个 `kind` 至多一份 extension 加一份 builtin，extension 生效 |
 * | `shortcut` | `id`、`title()`、`run(ctx)`，可选的 `keys`、`scope`、`enabled()` | 一条有快捷键的命令（`shortcuts.ts`）：设置里列出、可改键；一个键按下时焦点所在处的先于全局的，再按注册顺序；`id` 在全页唯一 |
 * | `conversation.view` | `id`、`title()`、`render(session)`，可选的 `match(session)`、`order` | 中间区里对话之外的一种视图：有会话打开、有注册方时顶上出现「对话 \| …」的切换，全部按 `order` 再按注册顺序；每个会话各记自己的选择 |
 *
 * 扩展还可以定义自己的点（`definePoint("<扩展 id>.<名>")`），别的包往里注册；注册进还没定义的点先挂着，全部注册完
 * （`settle`）还没人定义就丢掉并记进诊断，给了 `orElse` 的调它。一个扩展激活时注册的（`activate`）记在它名下，
 * 它停用时整批撤下（`withdraw`）；它的 `register` 返回的函数（订阅、计时器的收尾）这时一起调。
 *
 * 不是贡献点、但同样按 owner 记的还有三样：令牌覆盖（`tokens`）、语言包（`strings`）与图标（`icons`）。
 *
 * 注册不合形状的贡献被拒绝并记下理由；`match` 或 `render` 抛错只算这一个贡献失败，点照常工作。
 * 被拒的、失败的都列在诊断页里。
 */

import { createContext, createElement, Component, useContext, useSyncExternalStore, type ErrorInfo, type ReactNode } from "react";
import type { Block, BlockOf } from "@harnix/core/state";
import type { SessionRow } from "@harnix/core";
import type { ChatIds } from "./chat/types";
import type { SessionRef } from "./host";
import { addStrings, type Locale } from "./i18n/index";
import { FieldsSection, type FieldStore, type LocalField } from "./fields";
import { SIDEBAR_PRIORITIES, SIDEBAR_SCOPES, sameTier, type SidebarTabType } from "./sidebar";
import { SHORTCUT_SCOPES, parseKeys, type ShortcutCommand } from "./shortcuts";

export const POINTS = [
  "space",
  "home.card",
  "tool.row",
  "details.panel",
  "flow.block",
  "sidebar.group",
  "command",
  "settings.section",
  "session.action",
  "sidebar.tab",
  "shortcut",
  "conversation.view",
] as const;
export type Point = (typeof POINTS)[number];

/**
 * A view of the centre beside the conversation (a trajectory, say): the same session another way,
 * taking the conversation's place while it is chosen. The shell draws the switch at the top of the
 * centre whenever a session is open and at least one view applies; the conversation stays the
 * default, and each session remembers its own choice in this browser.
 */
export interface ConversationView {
  id: string;
  /** The switch's label. Read at every draw. */
  title(): string;
  /** Whether it applies to this session; absent, it applies to every one. A throw drops it. */
  match?(session: SessionRef): boolean;
  /** Where it goes among the views; 100 when absent. */
  order?: number;
  /** The centre while it is chosen. */
  render(session: SessionRef): ReactNode;
}

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

/**
 * A space's second side panel (xops-04 §7): the details of what is selected in its right column,
 * between the conversation and that column. See `SpaceContribution.secondary`.
 */
export interface SpaceSecondary {
  /** What is shown, as a stable key (a new key is a new thing shown: `data-key` on the panel). */
  key: string;
  /** The panel's heading. */
  title(): string;
  render(): ReactNode;
  /** The reader closed it: let go of what it showed (its selection), then `host.redraw()`. The
   *  conversation in the centre stays as it is. */
  onClose?(): void;
  /** Its width in px while the reader has not dragged it: within 300–760, 440 by default. */
  width?: number;
}

/** A space's right-column tabs, whichever way it gave them. */
export function tabsOf(space: SpaceContribution | undefined): SpaceTab[] {
  const tabs = space?.details?.tabs;
  return typeof tabs === "function" ? tabs() : (tabs ?? []);
}

/** A badge on a space's entry: a number or a very short text; nothing for null, undefined, 0, "". */
export type Badge = string | number | null | undefined;

/** What a session action is asked about: a session, one tool call in it, or a message a person sent
 *  in it (`turnId`: the turn it started; `text`: what it said). `space` is the interface space it is
 *  asked in (a space's `sessions` says which backend space that lists). */
export type ActionTarget =
  | { kind: "session"; session: SessionRef; space: string }
  | { kind: "tool"; session: SessionRef; space: string; toolUseId: string; tool: string; input?: unknown }
  | { kind: "message"; session: SessionRef; space: string; turnId: string; text: string };

/** Where the shell asks for session actions: the title bar, the session menu, a tool row — its
 *  buttons, and its argument (`arg`: the first action that matches makes the argument a link, its
 *  label the link's title; a file tool's path opening the file, say) — and a person's message's
 *  menu (`message`). */
export type ActionPlace = "header" | "menu" | "tool" | "arg" | "message";

/**
 * A space as the interface draws it (concept §5.4), its members grouped by where they are drawn.
 * Where its sessions land is the backend's. Every function is read at every draw: one whose answer
 * changes calls `host.redraw()`, or the space gives a `store` and the shell follows it.
 */
export interface SpaceContribution {
  id: string;
  /** A glyph name, or `<owner>.<name>` for an icon its owner registered. */
  icon: string;
  title(): string;
  order?: number;
  /** Where the left column lists it: with the product's spaces (`top`, the default) or at the
   *  bottom — for what is global rather than one of the surfaces. */
  place?: "top" | "bottom";
  /** The group it is listed in, among the `top` spaces: a line is drawn between two neighbours (by
   *  `order`) whose groups differ. Undefined counts as "". Grouping does not reorder. */
  group?: string;
  /** Another space's id: this one is shown inside it rather than listed — entered by code, and
   *  while it is in front that space's entry is lit. */
  within?: string;
  /** The backend space whose sessions this one lists and creates; its own id when absent. */
  sessions?: string;
  /** `host.navigate.open` brought this space to the front with a selection for it. */
  accept?(selection: unknown): void;
  /** The left column: the entry and what is under the space list. */
  sidebar?: {
    /** Under the space list: its entries. The space's own sessions when absent. */
    render?(): ReactNode;
    /** The left column's search box, drawn by the shell while this space is in front and gives
     *  `search`: its text as it changes ("" when cleared), and `submit` when Enter was pressed. */
    search?(q: string, submit?: boolean): void;
    /** With no `render()`: false leaves a session out of the space's list (say its main
     *  conversation, which its entry already opens). Absent, or a throw, lists it. */
    listSession?(session: SessionRef): boolean;
    /** A badge on its entry. */
    badge?(): Badge;
    /** A string: this space cannot be entered for now — its entry is greyed and says why on hover;
     *  `host.spaces.switch` to it does nothing. A throw counts as enabled. */
    disabled?(): string | false | null | undefined;
  };
  /** The centre. */
  center?: {
    /** The conversation, the overview with the conversation as a bar under it, or the overview alone. */
    mode?: "chat" | "minimized" | "hidden";
    /** The centre when it is not the conversation. */
    overview?(): ReactNode;
    /** The second side panel, between the conversation and the right column; null while there is
     *  nothing. Resizable, closable; on a narrow window it lies over the conversation. */
    secondary?(): SpaceSecondary | null | undefined;
    /** What the open session's conversation shows while it has no message yet. */
    emptyConversation?(session: SessionRef): ReactNode;
  };
  /** The conversation and its composer. */
  conversation?: {
    /** The open session is read-only: the words that take the composer's place. Only the
     *  interface — it guards against mistakes, it is not a security boundary. */
    readOnly?(session: SessionRef): string | false | undefined;
    /** What stays at the top of the open session's flow, above the messages. */
    pinned?(session: SessionRef): ReactNode;
    /** A line over the composer of an open session. */
    composerNotice?(session: SessionRef): ReactNode;
    /** Which words in the conversation's text are ids that open something. */
    ids?(): ChatIds | undefined;
    /** `steer`: a "deliver now" switch under the composer. */
    composer?: { steer?: boolean };
    /** Other owners' command names (without the `/`) left out of this space's `/` menu. */
    hideCommands?: string[];
    /** The `@` menu: whose candidates come first when it is cut to its length. */
    mention?: { first?: "contributed" | "files" };
    /** Content at the right end of the centre's title bar; `session` is the open one or null. */
    headerActions?(session: SessionRef | null): ReactNode;
    /** False: no way to start a conversation here (no "New session", no composer that creates one). */
    newSession?: boolean;
    /** What a conversation started here is created with (say, the project open in the space): read
     *  each time one is, under what the caller gave. */
    newSessionDefaults?(): { project_root?: string | null; scene?: string } | undefined;
    placeholder?(): string;
    /** A line under the title: what this space is for. */
    subtitle?(): string;
    /** What a new conversation here might start with, offered under the composer. */
    suggestions?(): string[];
    /** One line under a new conversation's composer (what the Agent may and may not do here). */
    notice?(): string;
  };
  /** The right column. */
  details?: {
    /** Tabs beside the details drawer; a function when they depend on what is selected. */
    tabs?: SpaceTab[] | (() => SpaceTab[]);
    /** The tab it holds current: one of its tab ids, or "details". */
    tab?(): string | undefined;
    /** Open or closed when the space is first entered; afterwards each space keeps its own. */
    initial?: "open" | "closed";
    /** Its width while the reader has not dragged it: px, or under 1 a share of the window. */
    width?: number;
    /** Its entry in the left column was clicked while it is already in front. */
    reenter?(): void;
  };
  /** A store-shaped value: when it changes the shell draws again, with no `redraw()` to remember. */
  store?: { subscribe(fn: () => void): () => void; getSnapshot(): unknown };
}

/** A session action: on a session, or a tool call in it. The shell asks it where `at` says. */
export interface SessionAction {
  id: string;
  at: ActionPlace[];
  icon?: string;
  /** The shell asks before running it. */
  danger?: boolean;
  label(target: ActionTarget): string;
  /** Read at every draw; a throw drops this one only. */
  match(target: ActionTarget): boolean;
  /** Shown but not for now: why, as its hover text (a turn running, say). Read at every draw; a
   *  throw counts as enabled. */
  disabled?(target: ActionTarget): string | false | null | undefined;
  run(target: ActionTarget): void | Promise<void>;
}

export interface Contributions {
  space: SpaceContribution;
  "home.card": { id: string; render(): ReactNode; title?(): string; order?: number };
  "tool.row": {
    id: string;
    match(block: BlockOf<"tool">): boolean;
    render(block: BlockOf<"tool">): ReactNode;
    /** The name the row gives the tool; the permission card asking about the call says the same. */
    title?(block: BlockOf<"tool">): string | null | undefined;
  };
  "details.panel": { id: string; match(item: DetailItem): boolean; render(item: DetailItem): ReactNode; title?(item: DetailItem): string };
  "flow.block": { id: string; match(block: Block): boolean; render(block: Block): ReactNode };
  "sidebar.group": { id: string; group(sessions: SessionRow[]): SidebarGroup[]; label?(): string };
  command: {
    id: string;
    /** Which leading character's menu asks it: `/` (the default) or `@`. */
    trigger?: "/" | "@";
    complete(query: string, ctx?: CommandContext): CommandCandidate[] | Promise<CommandCandidate[]>;
  };
  "settings.section": {
    id: string;
    label(): string;
    render(): ReactNode;
    /** What the shell may send a reader here for: `"models"` — a turn has no model to go to. */
    covers?: SettingsNeed[];
    /** The fields it declares, when it is drawn from them (a registration without `render`). */
    fields?: LocalField[];
    /** Where those fields' values are: this browser's (`harnix.pref.<owner>.<key>`) when none. */
    store?: FieldStore;
  };
  "session.action": SessionAction;
  "sidebar.tab": SidebarTabType;
  shortcut: ShortcutCommand;
  "conversation.view": ConversationView;
}

/** What a registration may give: a settings section may declare its fields in place of drawing itself. */
export type ContributionInput<P extends Point> = P extends "settings.section"
  ? Omit<Contributions["settings.section"], "render"> & ({ render(): ReactNode } | { render?: undefined; fields: LocalField[] })
  : Contributions[P];

/** What the shell itself sends a reader to the settings for, to whichever section `covers` it. */
export type SettingsNeed = "models";

/** What a point an extension defines is: how its contributions are used. */
export interface PointSpec {
  kind: "register" | "render" | "fold";
}

/** What a package's `register` (or an `orElse`) may answer: called when its batch is withdrawn —
 *  the topics it follows, its timers, stopped with what it registered. */
export type Dispose = () => void;

/** `register`'s options. */
export interface RegisterOptions {
  /** Called, in place of registering, when no one has defined the point by the time every
   *  registration is done: fill a place of your own instead. */
  orElse?: () => unknown;
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
  "session.action": ["label", "match", "run"],
  "sidebar.tab": ["title", "render"],
  shortcut: ["title", "run"],
  "conversation.view": ["title", "render"],
};

const ACTION_PLACES: readonly string[] = ["header", "menu", "tool", "arg", "message"];
const SPACE_GROUPS = ["sidebar", "center", "conversation", "details", "store"] as const;

const FIELD_TYPES: readonly string[] = ["toggle", "select", "number", "text", "secret", "list", "rules", "route"];

/** What is wrong with a settings section's `fields`; null when nothing. */
function fieldsError(fields: unknown): string | null {
  if (!Array.isArray(fields) || !fields.length) return "a settings section draws itself (`render`) or declares `fields`";
  for (const f of fields as Record<string, unknown>[]) {
    if (!f || typeof f.key !== "string" || !f.key || !FIELD_TYPES.includes(f.type as string)) {
      return `a settings field needs a \`key\` and a \`type\`: ${FIELD_TYPES.join(", ")}`;
    }
  }
  return null;
}

/** What is wrong with a kernel point's contribution beyond its functions; null when nothing. */
function shapeError(point: Point, c: Record<string, unknown>): string | null {
  if (point === "sidebar.tab") {
    if (typeof c.kind !== "string" || !c.kind) return "a tab type needs a `kind`";
    if (c.priority !== undefined && !(SIDEBAR_PRIORITIES as readonly unknown[]).includes(c.priority)) {
      return `a tab type's priority is ${SIDEBAR_PRIORITIES.join(", ")}, not ${JSON.stringify(c.priority)}`;
    }
    if (c.scope !== undefined && !(SIDEBAR_SCOPES as readonly unknown[]).includes(c.scope)) {
      return `a tab type's scope is ${SIDEBAR_SCOPES.join(" or ")}, not ${JSON.stringify(c.scope)}`;
    }
    if (c.patterns !== undefined && (!Array.isArray(c.patterns) || c.patterns.some((p) => typeof p !== "string" || !p))) {
      return "a tab type's `patterns` are strings";
    }
    if (c.guide !== undefined && (!Array.isArray(c.guide) || c.guide.some((g) => !g || typeof g !== "object" || typeof (g as { id?: unknown }).id !== "string" || typeof (g as { title?: unknown }).title !== "function"))) {
      return "a tab type's `guide` entries are `{ id, title() }`";
    }
  }
  if (point === "shortcut") {
    if (c.scope !== undefined && !(SHORTCUT_SCOPES as readonly unknown[]).includes(c.scope)) {
      return `a shortcut's scope is ${SHORTCUT_SCOPES.join(", ")}, not ${JSON.stringify(c.scope)}`;
    }
    const keys = c.keys;
    const written = typeof keys === "string" ? [keys] : keys && typeof keys === "object" ? Object.values(keys as Record<string, unknown>) : [];
    if ((keys !== undefined && keys !== null && typeof keys !== "string" && typeof keys !== "object") || written.some((k) => k !== null && k !== undefined && (typeof k !== "string" || !parseKeys(k, "mac")))) {
      return `a shortcut's keys are written like "Mod+K", not ${JSON.stringify(keys)}`;
    }
  }
  if (point === "command" && c.trigger !== undefined && c.trigger !== "/" && c.trigger !== "@") {
    return `a command's trigger is "/" or "@", not ${JSON.stringify(c.trigger)}`;
  }
  if (point === "session.action") {
    const at = c.at;
    if (!Array.isArray(at) || !at.length || at.some((x) => !ACTION_PLACES.includes(x as string))) {
      return `a session action's \`at\` lists where: ${ACTION_PLACES.join(", ")}`;
    }
  }
  if (point === "space") {
    // Contribution API 3 groups a space's members: a flat one is a module built for 2.
    const flat = SPACE_GROUPS.find((g) => c[g] !== undefined && (typeof c[g] !== "object" || c[g] === null || Array.isArray(c[g])));
    if (flat) return `a space's \`${flat}\` is a group of members (contribution API 3)`;
  }
  return null;
}

export interface Refusal {
  point: string;
  owner: string;
  id?: string;
  reason: string;
}

type Entry<P extends Point> = Contributions[P] & { owner: string };

/** What the registry keeps of a registration besides the contribution. */
interface Meta {
  /** The extension whose activation made it: what `withdraw` takes back. */
  batch: string | null;
  orElse?: () => unknown;
  /** `settle` has decided it: kept, or dropped into the diagnostics. */
  settled: boolean;
}

/** The extension points' definitions: who defined each, and how its contributions are used. */
interface Defined {
  owner: string;
  spec: PointSpec;
}

export class Registry {
  /** Every registration, by point name: the kernel's twelve, and whatever else was named — a point an
   *  extension defines, or one nobody has defined yet. */
  private readonly entries = new Map<string, Entry<Point>[]>(POINTS.map((p) => [p, []]));
  private readonly meta = new WeakMap<object, Meta>();
  private readonly defined = new Map<string, Defined>();
  /** The extension being activated (`activate`): what its registrations are kept under. */
  private batch: string | null = null;
  /** What each batch's `register` answered to be called when it is withdrawn. */
  private readonly disposers = new Map<string, Dispose[]>();
  private readonly overrides = new Map<string, { value: string; owner: string }>();
  private readonly glyphs = new Map<string, string>();
  private readonly refused: Refusal[] = [];
  /** The token names a module may override: the ones the page's stylesheets declare on `:root`.
   *  Set by the shell once its styles are in; until then every override waits. */
  private known: Set<string> | null = null;
  private pending: { tokens: Record<string, string>; owner: string }[] = [];
  private readonly listeners = new Set<() => void>();
  private version = 0;

  register<P extends Point>(point: P, contribution: ContributionInput<P>, owner: string, options?: RegisterOptions): boolean;
  /** Any other point: one an extension defines, or a name only known at run time. A kernel
   *  point's name goes through the typed overload above, so its shape is checked when compiled. */
  register<N extends string>(point: Exclude<N, Point>, contribution: unknown, owner: string, options?: RegisterOptions): boolean;
  register(point: string, contribution: unknown, owner: string, options: RegisterOptions = {}): boolean {
    const given = contribution as Record<string, unknown> | null;
    if (!given || typeof given !== "object" || typeof given.id !== "string") {
      return this.refuse({ point, owner, reason: "a contribution needs an `id`" });
    }
    let c: Record<string, unknown> & { id: string } = given as Record<string, unknown> & { id: string };
    if (point === "settings.section" && typeof c.render !== "function" && c.fields !== undefined) {
      const wrong = fieldsError(c.fields);
      if (wrong) return this.refuse({ point, owner, id: c.id, reason: wrong });
      const fields = c.fields as LocalField[];
      const store = c.store as FieldStore | undefined;
      c = { ...c, render: () => createElement(FieldsSection, { fields, store, owner }) };
    }
    const kernel = (POINTS as readonly string[]).includes(point);
    if (kernel) {
      const missing = SHAPE[point as Point].filter((name) => typeof c[name] !== "function");
      if (missing.length) {
        return this.refuse({ point, owner, id: c.id, reason: `missing ${missing.join(", ")}` });
      }
      const wrong = shapeError(point as Point, c);
      if (wrong) return this.refuse({ point, owner, id: c.id, reason: wrong });
      // A kind carries one extension type and one of the product's (`sidebar.ts`); a second at the
      // same tier is a clash, not an override — the later one is refused and the first stays in force.
      if (point === "sidebar.tab") {
        const type = c as unknown as SidebarTabType;
        const clash = this.at("sidebar.tab").find((held) => held.kind === type.kind && sameTier(held, type));
        if (clash) return this.refuse({ point, owner, id: c.id, reason: `\`${type.kind}\` already has a ${clash.priority ?? "extension"} type (${clash.id}, ${clash.owner})` });
      }
      // A command's id is what the person's own key for it is kept under: one command per id.
      if (point === "shortcut") {
        const held = this.at("shortcut").find((s) => s.id === c.id);
        if (held) return this.refuse({ point, owner, id: c.id, reason: `\`${c.id}\` is already a command (${held.owner})` });
      }
    }
    const entry = { ...(c as object), owner } as Entry<Point>;
    // A kernel point and one already defined take it as it is; any other waits for its definer.
    this.meta.set(entry, { batch: this.batch, orElse: options.orElse, settled: kernel || this.defined.has(point) });
    let list = this.entries.get(point);
    if (!list) this.entries.set(point, (list = []));
    list.push(entry);
    this.changed();
    return true;
  }

  /**
   * Define a contribution point of one's own, `<extension id>.<name>`: others register into it,
   * the definer reads it with `list` or `useContributions`. Inside `activate` the extension is the
   * one being activated; elsewhere `owner`. Refused, with why, for another's name, a kernel
   * point's, or a point already defined.
   */
  definePoint(name: string, spec: PointSpec, owner?: string): boolean {
    const by = this.batch ?? owner ?? "";
    if (!by || !name.startsWith(`${by}.`) || name.length === by.length + 1) {
      return this.refuse({ point: name, owner: by || "?", reason: `a point ${by || "it"} defines is \`${by || "<extension id>"}.<name>\`` });
    }
    if ((POINTS as readonly string[]).includes(name) || this.defined.has(name)) {
      return this.refuse({ point: name, owner: by, reason: `\`${name}\` is already defined` });
    }
    if (spec?.kind !== "register" && spec?.kind !== "render" && spec?.kind !== "fold") {
      return this.refuse({ point: name, owner: by, reason: "a point's kind is register, render or fold" });
    }
    this.defined.set(name, { owner: by, spec: { kind: spec.kind } });
    for (const entry of this.entries.get(name) ?? []) this.meta.get(entry)!.settled = true;
    this.changed();
    return true;
  }

  /** Whether a point is there to be filled: a kernel point, or one an extension defined. */
  isDefined(name: string): boolean {
    return (POINTS as readonly string[]).includes(name) || this.defined.has(name);
  }

  /** The points extensions defined, and who defined each. */
  definitions(): { name: string; owner: string; kind: PointSpec["kind"] }[] {
    return [...this.defined].map(([name, d]) => ({ name, owner: d.owner, kind: d.spec.kind }));
  }

  /** Every registration of a point, in order; none while the point is not defined. */
  list<T = unknown>(name: string): (T & { owner: string; id: string })[] {
    if (!this.isDefined(name)) return [];
    return (this.entries.get(name) ?? []) as unknown as (T & { owner: string; id: string })[];
  }

  /**
   * Run one extension's `register`: everything registered and defined in it is kept under `ext`,
   * and goes with it at `withdraw(ext)`, and so does the function it answers, if any (a `Dispose`)
   * — called then. Synchronous: what it registers later, after an `await`, is under the owner it
   * names only.
   */
  activate(ext: string, register: () => unknown): void {
    const outer = this.batch;
    this.batch = ext;
    try {
      const dispose = register();
      if (typeof dispose === "function") {
        let list = this.disposers.get(ext);
        if (!list) this.disposers.set(ext, (list = []));
        list.push(dispose as Dispose);
      }
    } finally {
      this.batch = outer;
    }
  }

  /**
   * Every registration is in: one into a point still not defined is dropped — its `orElse` called
   * instead (inside the same extension's batch), or the diagnostics told. The shell calls it after
   * activating at hello, and again after each later activation.
   */
  settle(): void {
    for (const [name, list] of this.entries) {
      if (this.isDefined(name)) continue;
      for (const entry of [...list]) {
        const meta = this.meta.get(entry)!;
        if (meta.settled) continue;
        meta.settled = true;
        list.splice(list.indexOf(entry), 1);
        if (meta.orElse) {
          const orElse = meta.orElse;
          try {
            this.activate(meta.batch ?? entry.owner, orElse);
          } catch (e) {
            this.fail(name, entry, e);
          }
        } else {
          this.refuse({ point: name, owner: entry.owner, id: entry.id, reason: `no one defines the point \`${name}\`` });
        }
      }
    }
    this.changed();
  }

  /**
   * Take back everything one extension's activation registered, and every registration owned by
   * it: its points' definitions with them (what others registered into those stays, unseen, until
   * they are defined again), its icons, its token overrides; and call what its `register` answered.
   * Answers how many went.
   */
  withdraw(ext: string): number {
    for (const dispose of this.disposers.get(ext) ?? []) {
      try {
        dispose();
      } catch (e) {
        this.fail("module", { id: "dispose", owner: ext }, e);
      }
    }
    this.disposers.delete(ext);
    let gone = 0;
    for (const list of this.entries.values()) {
      for (let i = list.length - 1; i >= 0; i -= 1) {
        const entry = list[i]!;
        if (entry.owner === ext || this.meta.get(entry)?.batch === ext) {
          list.splice(i, 1);
          gone += 1;
        }
      }
    }
    for (const [name, d] of [...this.defined]) if (d.owner === ext) this.defined.delete(name);
    for (const name of [...this.glyphs.keys()]) if (name.startsWith(`${ext}.`)) this.glyphs.delete(name);
    for (const [name, o] of [...this.overrides]) if (o.owner === ext) this.overrides.delete(name);
    this.changed();
    return gone;
  }

  /**
   * Take back one registration: `owner`'s `id` at `point`. For what the shell registers on an
   * package's behalf from its manifest (its declared inline points), which goes when the
   * package stops or changes. Answers whether there was one.
   */
  unregister(point: Point, id: string, owner: string): boolean {
    const list = this.entries.get(point);
    const at = list?.findIndex((c) => c.id === id && c.owner === owner) ?? -1;
    if (!list || at < 0) return false;
    list.splice(at, 1);
    this.changed();
    return true;
  }

  /**
   * Token overrides (harnix-ext.md §22.4.1): an extension's colours for names the theme already has. A name the
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

  /** An extension's language pack; keys outside `<owner>.` are refused (harnix-ext.md §22.4.1). */
  strings(locale: Locale, dictionary: Record<string, string>, owner: string): void {
    for (const key of addStrings(locale, dictionary, owner)) {
      this.refuse({ point: "strings", owner, id: key, reason: `a key of ${owner}'s pack is \`${owner}.…\`` });
    }
    this.changed();
  }

  /**
   * An extension's own icons: name → the inside of an SVG (no `<svg>`), drawn on the shell's
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

  /** An extension icon's inside, as `icons` took it; undefined for a name it does not have. */
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

  /** The session actions for a place and a target, in order: those whose `at` has the place and
   *  whose `match` says yes. One whose `match` throws is left out, and counted as failed. */
  actions(place: ActionPlace, target: ActionTarget): Entry<"session.action">[] {
    return this.at("session.action").filter((a) => {
      if (!a.at.includes(place)) return false;
      try {
        return a.match(target) === true;
      } catch (e) {
        queueMicrotask(() => this.fail("session.action", a, e));
        return false;
      }
    });
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

const RegistryContext = createContext<Registry | null>(null);

/** The registry the page's UI packages registered into, for `useContributions`. */
export function RegistryProvider({ registry, children }: { registry: Registry; children: ReactNode }) {
  return createElement(RegistryContext.Provider, { value: registry }, children);
}

/** A point's registrations, kept up to date: what the point's definer draws from. */
export function useContributions<T = unknown>(name: string, registry?: Registry): (T & { owner: string; id: string })[] {
  const given = useContext(RegistryContext);
  const r = registry ?? given;
  if (!r) throw new Error("@harnix/react: useContributions() needs a <RegistryProvider> above it, or a registry");
  useSyncExternalStore(r.subscribe, r.getVersion);
  return r.list<T>(name);
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
