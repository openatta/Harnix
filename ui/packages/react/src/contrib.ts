/**
 * 贡献点注册表（概念 §5.1、设计 §10.3）：界面上哪些地方可以由内建或运行期包填，以及怎么填。
 *
 * v2 的八个点，都注册 React 组件：
 *
 * | 点 | 形状 | 怎么用 |
 * |---|---|---|
 * | `space` | `id`、`icon`、`title()`、`center`、`sidebar?()`、`overview?()`、`tabs?` | 全部，按 `order` 再按注册顺序 |
 * | `home.card` | `id`、`render()`、`title?()` | 全部，按 `order` 再按注册顺序 |
 * | `tool.row` | `match(tool块)`、`render(tool块)` | 后注册的先认领 |
 * | `details.panel` | `match(item)`、`render(item)`、`title?(item)` | 后注册的先认领 |
 * | `flow.block` | `match(块)`、`render(块)` | 后注册的先认领 |
 * | `sidebar.group` | `group(会话) → 组[]`、`label?()` | 只用最后注册的那一个 |
 * | `command` | `complete(query) → 候选[]` | 全部，按注册顺序拼起来 |
 * | `settings.section` | `id`、`label()`、`render()` | 全部，按注册顺序 |
 *
 * 注册不合形状的贡献被拒绝并记下理由；`match` 或 `render` 抛错只算这一个贡献失败，点照常工作。
 * 被拒的、失败的都列在诊断页里。
 */

import { createElement, Component, useSyncExternalStore, type ErrorInfo, type ReactNode } from "react";
import type { Block, BlockOf } from "@harnix/core/state";
import type { SessionRow } from "@harnix/core";

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
}

/** A space as the interface draws it (design §8.1). Where its sessions land is the backend's. */
export interface SpaceContribution {
  id: string;
  /** A glyph name. */
  icon: string;
  title(): string;
  order?: number;
  /** What the centre shows: the conversation, the overview with the conversation as a bar under
   *  it, or the overview alone. */
  center?: "chat" | "minimized" | "hidden";
  /** The left column below the space list: its entries. The space's own sessions when absent. */
  sidebar?(): ReactNode;
  /** The centre when it is not the conversation. */
  overview?(): ReactNode;
  /** Right-column tabs, beside the details drawer. */
  tabs?: { id: string; title(): string; render(): ReactNode }[];
  /** A line under the title: what this space is for. */
  subtitle?(): string;
  /** What a new conversation here might start with, offered under the composer. */
  suggestions?(): string[];
  /** One line under the composer (what the Agent may and may not do here). */
  notice?(): string;
  placeholder?(): string;
}

export interface Contributions {
  space: SpaceContribution;
  "home.card": { id: string; render(): ReactNode; title?(): string; order?: number };
  "tool.row": { id: string; match(block: BlockOf<"tool">): boolean; render(block: BlockOf<"tool">): ReactNode };
  "details.panel": { id: string; match(item: DetailItem): boolean; render(item: DetailItem): ReactNode; title?(item: DetailItem): string };
  "flow.block": { id: string; match(block: Block): boolean; render(block: Block): ReactNode };
  "sidebar.group": { id: string; group(sessions: SessionRow[]): SidebarGroup[]; label?(): string };
  command: { id: string; complete(query: string): CommandCandidate[] | Promise<CommandCandidate[]> };
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
  private readonly refused: Refusal[] = [];
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
    list.push({ ...(c as object), owner } as Entry<Point>);
    this.changed();
    return true;
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
    if (!this.refused.some((r) => r.point === refusal.point && r.owner === refusal.owner && r.id === refusal.id && r.reason === refusal.reason)) {
      this.refused.push(refusal);
    }
    this.changed();
    return false;
  }

  private changed(): void {
    this.version += 1;
    for (const fn of [...this.listeners]) fn();
  }
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
