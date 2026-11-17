/**
 * 失败的归类：一轮失败（`harnix.turn_settled` 的 `error`）、发送被拒，引擎在 `error.data.kind` 里
 * 给了原因（`auth`、`rate_limited`……），宿主自己的码（实例不可达、扩展不可用）另有一组。
 * 归成界面有话说的几类；认不得的不归类，界面照原话。码与原话不丢，放在提示（`detail`）里给支持看。
 */

import { errorText } from "../client/index.ts";
import { DEADLINE_EXCEEDED, EXTENSION_UNAVAILABLE, INSTANCE_UNAVAILABLE } from "../protocol/index.ts";

export type FailureClass =
  | "no_model"
  | "auth"
  | "billing"
  | "model_rejected"
  | "rate_limited"
  | "overloaded"
  | "model_unavailable"
  | "context_too_long"
  | "egress_denied"
  | "engine_down"
  | "instance_unavailable"
  | "extension_unavailable"
  | "deadline";

/** 能直接带人去处理的类：说完之后给一个去设置 · 模型与凭据的入口。 */
export type FailureAction = "settings.models";

export interface FailureFacts {
  /** 错误的原话（引擎或宿主的 `message`） */
  raw: string;
  code?: number;
  /** 引擎的 `data.kind` */
  kind?: string;
  retryAfterMs?: number;
}

/** 一个 reject 出来的错误，取出归类要看的几样；不是对象的只有原话。 */
export function failureFacts(e: unknown): FailureFacts {
  const record = e && typeof e === "object" ? (e as { code?: unknown; message?: unknown; data?: unknown }) : {};
  const data = record.data && typeof record.data === "object" ? (record.data as { kind?: unknown; retry_after_ms?: unknown }) : {};
  return {
    raw: errorText(e),
    code: typeof record.code === "number" ? record.code : undefined,
    kind: typeof data.kind === "string" ? data.kind : undefined,
    retryAfterMs: typeof data.retry_after_ms === "number" ? data.retry_after_ms : undefined,
  };
}

const KINDS: Record<string, FailureClass> = {
  // 引擎拿不到这个 provider 的密钥，什么也没发出去。
  no_credentials: "no_model",
  auth: "auth",
  billing: "billing",
  model_rejected: "model_rejected",
  rate_limited: "rate_limited",
  overloaded: "overloaded",
  model_unavailable: "model_unavailable",
  context_too_long: "context_too_long",
  egress_denied: "egress_denied",
  engine_stopped: "engine_down",
  engine_unreachable: "engine_down",
};

/** 没有配模型服务或密钥时引擎的原话（`missing api_key`），归 `no_model`，不论它标的是哪一种；引擎自己标的是 `no_credentials`。 */
const NO_MODEL = /missing api_key|no api[ _-]?key|no provider|no model|not configured/i;

/** 账户没钱时服务的原话（Anthropic 答 400 `credit balance is too low`，引擎归成 `model_rejected`）：归 `billing`。 */
const BILLING = /credit balance|billing/i;

export function failureClass(f: FailureFacts): FailureClass | undefined {
  if (NO_MODEL.test(f.raw)) return "no_model";
  if (BILLING.test(f.raw)) return "billing";
  if (f.kind && KINDS[f.kind]) return KINDS[f.kind];
  if (f.code === INSTANCE_UNAVAILABLE) return "instance_unavailable";
  if (f.code === EXTENSION_UNAVAILABLE) return "extension_unavailable";
  if (f.code === DEADLINE_EXCEEDED) return "deadline";
  return undefined;
}

export function failureAction(cls: FailureClass): FailureAction | undefined {
  return cls === "no_model" || cls === "auth" || cls === "billing" || cls === "model_rejected" ? "settings.models" : undefined;
}

/** 服务自己的说明至多留这么多字，多的截掉、带省略号。 */
export const SERVICE_SAID_MAX = 160;

/**
 * 限流时服务自己的说明（哪个额度、几点恢复）的摘要，给界面跟在归类的话后面；没有就是 `undefined`。
 * 引擎把它接在 `<kind>` 或 `<kind> (HTTP n)` 加冒号后面（可能不止一层）；整段是 JSON 的（兼容 OpenAI 的服务）取 `error.message`。
 * 压成一行、去掉控制字符，至多 `SERVICE_SAID_MAX` 字——是界面当文字画的，不当标记。
 */
export function serviceSaid(f: FailureFacts): string | undefined {
  if (f.kind !== "rate_limited") return undefined;
  // 引擎给的消息里这个前缀会叠两层（`rate_limited: rate_limited: …`），有几层去几层。
  const prefix = new RegExp(`^(?:${f.kind}(?:\\s*\\(HTTP \\d+\\))?:\\s*)+`);
  if (!prefix.test(f.raw)) return undefined;
  let said = f.raw.replace(prefix, "");
  if (said.startsWith("{")) {
    try {
      const body = JSON.parse(said) as { error?: { message?: unknown } | string; message?: unknown };
      const inner = typeof body.error === "object" && body.error ? body.error.message : body.error ?? body.message;
      if (typeof inner === "string") said = inner;
    } catch {
      // 被引擎截断了，或其实不是 JSON：照原文。
    }
  }
  said = said.replace(/[\u0000-\u001f\u007f\s]+/g, " ").trim();
  if (!said) return undefined;
  const chars = Array.from(said);
  return chars.length > SERVICE_SAID_MAX ? `${chars.slice(0, SERVICE_SAID_MAX - 1).join("").trimEnd()}…` : said;
}

/** 给提示用：原话，加上码（有就带）。 */
export function failureDetail(f: FailureFacts): string {
  const tags = [f.code !== undefined ? String(f.code) : "", f.kind ?? ""].filter(Boolean).join(" ");
  return tags ? `${f.raw} (${tags})` : f.raw;
}
