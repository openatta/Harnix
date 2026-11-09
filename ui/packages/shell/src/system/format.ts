/** How the system space writes figures and labels: sizes, times, tokens, a message's text. */

import { clip, t } from "@harnix/react";

import type { AgentUsage, Task, TraceCall } from "./types.ts";

export function uptime(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return h ? `${h}:${String(m).padStart(2, "0")}:${String(secs % 60).padStart(2, "0")}` : `${m}:${String(secs % 60).padStart(2, "0")}`;
}

export function totalTokens(u: AgentUsage): number {
  return u.input_tokens + u.cache_read_tokens + u.cache_write_tokens + u.output_tokens;
}

export function tokenTip(u: AgentUsage[]): string {
  const sum = (k: keyof AgentUsage) => u.reduce((n, a) => n + Number(a[k] ?? 0), 0);
  return t("system.tokenTip", { input: sum("input_tokens"), read: sum("cache_read_tokens"), write: sum("cache_write_tokens"), output: sum("output_tokens") });
}

export function ms(n: number): string {
  if (n < 1000) return `${n}ms`;
  const secs = n / 1000;
  return secs < 60 ? `${secs.toFixed(1)}s` : `${Math.floor(secs / 60)}m${String(Math.round(secs % 60)).padStart(2, "0")}s`;
}

export function tokens(usage: TraceCall["usage"]): string {
  if (!usage) return "—";
  const input = (usage.input_tokens ?? 0) + (usage.cache_read_input_tokens ?? 0) + (usage.cache_creation_input_tokens ?? 0);
  return `in ${compact(input)} · out ${compact(usage.output_tokens ?? 0)}`;
}

export function compact(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
}

export function originLabel(origin: unknown): string {
  if (typeof origin === "string") return origin;
  if (origin && typeof origin === "object") {
    const [kind, value] = Object.entries(origin as Record<string, unknown>)[0] ?? [];
    return kind ? `${kind}:${String(value)}` : "";
  }
  return "";
}

export function messageText(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .map((block: { type?: string; text?: string; content?: unknown; name?: string }) => {
      if (block.type === "text") return block.text ?? "";
      if (block.type === "tool_result") return messageText(block.content);
      if (block.type === "tool_use") return `⚙ ${block.name ?? ""}`;
      if (block.type === "image") return "▣";
      return "";
    })
    .filter(Boolean)
    .join(" ");
}

export function isPlainText(content: unknown): boolean {
  return typeof content === "string" || (Array.isArray(content) && content.every((b: { type?: string }) => b.type === "text"));
}

export function taskLabel(task: Task): string {
  const d = task.detail ?? {};
  const text = d.description ?? d.command ?? d.prompt ?? d.member ?? d.cron ?? task.task_id;
  return clip(String(text), 120);
}

export function duration(task: Task): string {
  if (!task.started_ms) return "";
  const secs = Math.max(0, Math.round(((task.ended_ms ?? Date.now()) - task.started_ms) / 1000));
  return secs < 60 ? `${secs}s` : `${Math.floor(secs / 60)}m${String(secs % 60).padStart(2, "0")}s`;
}
