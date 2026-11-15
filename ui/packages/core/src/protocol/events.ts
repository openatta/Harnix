/**
 * 会话事件的解析视图，与认得的引擎协议。
 *
 * `harnix.event` 的 `event` 是引擎（Agix）的形状，不在 `PROTOCOL_VERSION` 的加法承诺里，随 `hello.engine_protocol` 变。
 * 只有这一层认识它：别处读 `ParsedSessionEvent`，引擎改了形状只改这里。不认识的 `engine_protocol` 由客户端拒绝并说明，
 * 而不是把读不懂的事件画成别的东西。
 */

import type { EngineEvent, Hello } from "./frames.ts";

/** 这一层读得懂的引擎协议版本（`hello.engine_protocol`）。 */
export const ENGINE_PROTOCOLS: readonly number[] = [4];

/** 不认识的引擎协议：为什么拒绝；认识（或没说）时 null。 */
export function engineProtocolRefusal(hello: Pick<Hello, "engine_protocol"> | null | undefined): string | null {
  const v = hello?.engine_protocol;
  if (v === undefined || ENGINE_PROTOCOLS.includes(v)) return null;
  return `the engine speaks protocol ${String(v)}; this interface reads ${ENGINE_PROTOCOLS.join(", ")} — use an interface built for this Harnix`;
}

/** 一个会话事件，照它的意思读出来；认不出的是 `{ kind: "other" }`。 */
export type ParsedSessionEvent =
  | { kind: "user_message"; text: string }
  | { kind: "text"; text: string }
  | { kind: "thinking"; text: string }
  | { kind: "tool_started"; toolUseId: string; tool: string }
  | { kind: "tool_called"; toolUseId: string; tool: string; input: unknown }
  | { kind: "tool_result"; toolUseId: string; isError: boolean }
  | { kind: "prompt"; promptId: string }
  | { kind: "turn_complete"; stopReason: string | null }
  | { kind: "error"; message: string }
  | { kind: "other"; raw: string };

const str = (v: unknown): string => (typeof v === "string" ? v : "");

/** 解析一个引擎事件。 */
export function parseSessionEvent(raw: EngineEvent | unknown): ParsedSessionEvent {
  const e = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
  const kind = str(e.kind);
  switch (kind) {
    case "user_message":
      return { kind, text: str(e.text) };
    case "text_delta":
      return { kind: "text", text: str(e.text) };
    case "thinking_delta":
      return { kind: "thinking", text: str(e.text) };
    case "tool_use_start":
      return { kind: "tool_started", toolUseId: str(e.id), tool: str(e.name) };
    case "tool_use":
      return { kind: "tool_called", toolUseId: str(e.id), tool: str(e.name), input: e.input ?? null };
    case "tool_result":
      return { kind, toolUseId: str(e.id), isError: e.is_error === true };
    case "prompt":
      return { kind, promptId: str(e.prompt_id) };
    case "turn_complete":
      return { kind, stopReason: typeof e.stop_reason === "string" ? e.stop_reason : null };
    case "error":
      return { kind, message: str(e.message) };
    default:
      return { kind: "other", raw: kind };
  }
}
