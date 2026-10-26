/**
 * 会话组件的小工具：时间、耗时、token 数、编号变 chip、工具行的名字 / 参数 / 结果摘要 / diff。
 * 纯函数，不依赖 React；应用可以单独拿去用。
 */

import { escapeHtml } from "../markdown";
import { lineDiff, type DiffLine } from "../diff";
import type { BlockOf } from "@harnix/core/state";

/* ── 时间与数 ──────────────────────────────────────────────────────── */

/** 消息旁的时间：今天的只写时分。 */
export function clockTime(at: number | undefined): string {
  if (at == null) return "";
  const d = new Date(at);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `${hh}:${mm}`;
}

/** 耗时：0.8s、12s、1m 3s。 */
export function duration(ms: number): string {
  if (ms < 0) return "";
  if (ms < 10_000) return `${(ms / 1000).toFixed(1)}s`;
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m ${s % 60}s`;
}

/** 96k、1.2M。 */
export function compactTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1000) return `${Math.round(n / 1000)}k`;
  return String(n);
}

/* ── 编号变 chip ───────────────────────────────────────────────────── */

/**
 * 文本里的编号（应用给的正则，必须带 g 标志）切成段：`{text}` 原样、`{id}` 渲染成 chip。
 * 用于人的消息（纯文本）。
 */
export function splitIds(text: string, pattern: RegExp | undefined): ({ text: string } | { id: string })[] {
  if (!pattern) return [{ text }];
  const re = new RegExp(pattern.source, pattern.flags.includes("g") ? pattern.flags : `${pattern.flags}g`);
  const out: ({ text: string } | { id: string })[] = [];
  let last = 0;
  for (const m of text.matchAll(re)) {
    const at = m.index ?? 0;
    if (at > last) out.push({ text: text.slice(last, at) });
    out.push({ id: m[0] });
    last = at + m[0].length;
  }
  if (last < text.length) out.push({ text: text.slice(last) });
  return out;
}

/**
 * markdown 渲染出的 HTML 里的编号变成 chip：只动标签之外的文字，跳过 code、pre、a 里面的。
 * chip 的文字就是编号本身，所以消息的 textContent 不变。HTML 已转义过，编号的字符集（字母、数字、-）
 * 不会碰到实体。
 */
export function chipifyHtml(html: string, pattern: RegExp | undefined, clickable: boolean | ((id: string) => boolean)): string {
  if (!pattern) return html;
  const re = new RegExp(pattern.source, pattern.flags.includes("g") ? pattern.flags : `${pattern.flags}g`);
  let skip = 0;
  return html
    .split(/(<[^>]+>)/)
    .map((part) => {
      if (part.startsWith("<")) {
        const tag = /^<\/?\s*(\w+)/.exec(part)?.[1]?.toLowerCase();
        if (tag === "code" || tag === "pre" || tag === "a") skip += part.startsWith("</") ? -1 : 1;
        return part;
      }
      if (skip > 0 || !part) return part;
      return part.replace(re, (id) => chipHtml(id, typeof clickable === "function" ? clickable(id) : clickable));
    })
    .join("");
}

function chipHtml(id: string, clickable: boolean): string {
  const safe = escapeHtml(id);
  const interactive = clickable ? ` role="link" tabindex="0"` : "";
  return `<span class="hx-chip${clickable ? " hx-chip-link" : ""}" data-testid="conv-chip" data-id="${safe}"${interactive}>${safe}</span>`;
}

/* ── 工具行 ───────────────────────────────────────────────────────── */

/** 内建的友好名字（编程助手的习惯：Edit 叫 Update）。应用可经 ChatPanel 的 toolNames 覆盖或补充。 */
export const TOOL_NAMES: Record<string, string> = {
  Bash: "Bash",
  Read: "Read",
  Edit: "Update",
  MultiEdit: "Update",
  Write: "Write",
  NotebookEdit: "Update",
  Grep: "Search",
  Glob: "Search",
  LS: "List",
  WebFetch: "Fetch",
  WebSearch: "Web",
  Task: "Agent",
  TodoWrite: "Todo",
};

/** 工具的显示名：先查表（也认以 * 结尾的前缀，如 "xflow_*"）；MCP 工具（mcp__服务__工具）显示工具名本身。 */
export function toolDisplayName(name: string, names: Record<string, string> = TOOL_NAMES): string {
  const look = (n: string): string | undefined => {
    if (names[n]) return names[n];
    for (const [k, v] of Object.entries(names)) if (k.endsWith("*") && n.startsWith(k.slice(0, -1))) return v;
    return undefined;
  };
  const direct = look(name);
  if (direct) return direct;
  const mcp = /^mcp__[^_]+(?:_[^_]+)*?__(.+)$/.exec(name);
  if (mcp) return look(mcp[1]!) ?? mcp[1]!;
  return name;
}

function oneLine(text: string, n = 160): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > n ? `${flat.slice(0, n - 1)}…` : flat;
}

/** 参数摘要：最值得看的那个参数；没有就把对象写成 `k: v` 的一行。 */
export function toolArgs(input: unknown): string {
  if (input == null) return "";
  if (typeof input === "string") return oneLine(input);
  if (typeof input !== "object") return oneLine(String(input));
  const record = input as Record<string, unknown>;
  for (const key of ["command", "file_path", "notebook_path", "path", "pattern", "query", "url", "id", "name", "description", "prompt"]) {
    const v = record[key];
    if (typeof v === "string" && v) return oneLine(v);
  }
  const parts = Object.entries(record)
    .filter(([k]) => !k.startsWith("_"))
    .map(([k, v]) => `${k}: ${typeof v === "string" ? v : JSON.stringify(v)}`);
  return oneLine(parts.join(", "));
}

/** 编辑类工具的新旧文本：Edit / MultiEdit / Write（以及形状相同的别名）。没有就是 null。 */
export function editOf(block: BlockOf<"tool">): { path: string; pairs: { old: string; new: string }[] } | null {
  const input = block.input as Record<string, unknown> | undefined;
  if (!input || typeof input !== "object") return null;
  const path = String(input.file_path ?? input.path ?? input.notebook_path ?? "");
  if (typeof input.old_string === "string" && typeof input.new_string === "string") {
    return { path, pairs: [{ old: input.old_string, new: input.new_string }] };
  }
  if (Array.isArray(input.edits)) {
    const pairs = (input.edits as { old_string?: unknown; new_string?: unknown }[])
      .filter((e) => typeof e?.old_string === "string" && typeof e?.new_string === "string")
      .map((e) => ({ old: e.old_string as string, new: e.new_string as string }));
    return pairs.length ? { path, pairs } : null;
  }
  if (typeof input.content === "string" && /write/i.test(block.name)) {
    return { path, pairs: [{ old: "", new: input.content }] };
  }
  if (typeof input.new_source === "string") return { path, pairs: [{ old: "", new: input.new_source }] };
  return null;
}

export interface NumberedDiffLine {
  op: DiffLine["op"];
  text: string;
  /** 旧文本里的行号（新增行没有） */
  oldNo?: number;
  /** 新文本里的行号（删除行没有） */
  newNo?: number;
}

/** 带行号的 diff（行号相对于这段改动，不知道它在文件里的位置）。 */
export function numberedDiff(oldText: string, newText: string): NumberedDiffLine[] {
  const lines = oldText === "" ? newText.split("\n").map((text) => ({ op: "add" as const, text })) : lineDiff(oldText, newText);
  let o = 1;
  let n = 1;
  return lines.map((l) => {
    if (l.op === "add") return { ...l, newNo: n++ };
    if (l.op === "del") return { ...l, oldNo: o++ };
    return { ...l, oldNo: o++, newNo: n++ };
  });
}

export interface ToolSummaryTexts {
  running: string;
  interrupted: string;
  done: string;
  readLines: (n: number) => string;
  editLines: (added: number, removed: number) => string;
  wroteLines: (n: number) => string;
  results: (n: number) => string;
}

/** "⎿" 后面那一行：结果的一句话。 */
export function toolSummary(block: BlockOf<"tool">, t: ToolSummaryTexts): string {
  if (block.status === "running") return t.running;
  if (block.status === "interrupted") return t.interrupted;
  const result = block.result ?? "";
  const lines = result.split("\n").filter((l) => l.trim());
  if (block.status === "error") return oneLine(lines[0] ?? t.interrupted, 200);
  const edit = editOf(block);
  if (edit) {
    let added = 0;
    let removed = 0;
    for (const p of edit.pairs) {
      for (const l of numberedDiff(p.old, p.new)) {
        if (l.op === "add") added += 1;
        else if (l.op === "del") removed += 1;
      }
    }
    return edit.pairs.every((p) => p.old === "") ? t.wroteLines(added) : t.editLines(added, removed);
  }
  const base = block.name.replace(/^mcp__.*__/, "");
  if (base === "Read") return t.readLines(result ? result.split("\n").length : 0);
  if (base === "Grep" || base === "Glob") return lines.length ? t.results(lines.length) : t.results(0);
  if (!lines.length || (lines.length === 1 && /^\(no output\)$/i.test(lines[0]!.trim()))) return t.done;
  const json = jsonSummary(result);
  if (json) return json;
  // Bash 之类：最后一行最说明问题（测试结果、构建结果）；其余取第一行
  const pick = base === "Bash" ? lines[lines.length - 1]! : lines[0]!;
  return oneLine(pick, 200);
}

/** 结果是 JSON（MCP 工具常见）：挑几个说明问题的字段写成一行。 */
function jsonSummary(text: string): string | null {
  const trimmed = text.trim();
  if (!/^[[{]/.test(trimmed)) return null;
  let value: unknown;
  try {
    value = JSON.parse(trimmed);
  } catch {
    return null;
  }
  if (Array.isArray(value)) return `[${value.length}]`;
  if (!value || typeof value !== "object") return null;
  const record = value as Record<string, unknown>;
  const picked = ["kind", "id", "status", "title", "summary", "message", "stage"]
    .filter((k) => record[k] != null && typeof record[k] !== "object")
    .map((k) => `${k}: ${String(record[k])}`);
  if (picked.length) return oneLine(picked.join(" · "), 200);
  return oneLine(JSON.stringify(value), 200);
}
