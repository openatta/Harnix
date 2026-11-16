/**
 * 一个提问要人答的是什么：权限、计划审阅、模型的提问、MCP server 的询问。
 *
 * 引擎的帧（`prompt`，daemon_rpc_protocol §7）在这里读成 `AskForm`，界面只画它，不认帧的形状。
 * 回答走 `harnix.session.respond`：权限与计划给 `decision`，提问与询问给 `answer`（`AskReply`）。
 */

import type { AskAnswered, AskReply, Decision, PromptEvent } from "../protocol/index.ts";

/** 计划审阅是离开计划模式那个工具的权限询问：批准就离开，要求修改就留在计划模式、把意见交给模型。 */
export const PLAN_TOOL = "ExitPlanMode";

export interface AskChoice {
  key: string;
  label: string;
}

/** 计划要的放行：一类动作（「跑测试」），不是一条具体的命令。 */
export interface PlanAllowance {
  tool: string;
  prompt: string;
}

/**
 * MCP server 的表单里的一个字段。只画原始类型（文字、数、整数、是否、选一个）；别的是 `unsupported`，
 * 必填的有这种字段时表单交不了，只能拒绝或不选。
 */
export interface ElicitField {
  name: string;
  /** 给人看的名字：schema 的 `title`，没有就是字段名。 */
  title: string;
  description?: string;
  required: boolean;
  kind: "text" | "number" | "integer" | "boolean" | "choice" | "unsupported";
  /** `choice` 的选项：`value` 是交回去的值。 */
  choices?: AskChoice[];
  /** `text` 的格式（`email`、`uri`、`date`、`date-time`）。 */
  format?: string;
  default?: string | number | boolean;
  minimum?: number;
  maximum?: number;
  minLength?: number;
  maxLength?: number;
}

export type AskForm =
  | { type: "permission" }
  /** `plan` 是模型在那次调用之前的回复（它就是要审的计划）；没找到时为空。 */
  | { type: "plan"; plan: string; allowed: PlanAllowance[] }
  /** 没有选项就是自由回答；有选项时也可以用自己的话答。 */
  | { type: "question"; question: string; header: string | null; options: AskChoice[] }
  /**
   * `server` 写的东西（`message`、字段、网址）都标明出处。`url` 模式不带字段：人自己去那个页面，回来说「好了」；
   * 网址完整显示，连同主机，不自动打开。
   */
  | { type: "elicitation"; server: string; message: string; limited: boolean; fields: ElicitField[]; url?: { href: string; host: string } };

/** 读一个提问：`before` 是它之前的块（计划审阅从里面找计划与那次调用）。 */
export function askForm(
  prompt: PromptEvent,
  before: readonly { kind: string; text?: string; id?: string; input?: unknown }[],
): AskForm {
  switch (prompt.prompt_type) {
    case "question":
      return {
        type: "question",
        question: String(prompt.question ?? prompt.message ?? ""),
        header: typeof prompt.header === "string" && prompt.header ? prompt.header : null,
        options: (prompt.options ?? []).filter((o) => o && typeof o.key === "string").map((o) => ({ key: o.key, label: String(o.label ?? o.key) })),
      };
    case "mcp_elicitation": {
      const url = prompt.mode === "url" && prompt.url ? { href: String(prompt.url), host: String(prompt.url_host ?? "") } : undefined;
      return {
        type: "elicitation",
        server: String(prompt.server ?? ""),
        message: String(prompt.message ?? ""),
        limited: prompt.limited === true,
        fields: url ? [] : elicitationFields(prompt.requested_schema),
        ...(url ? { url } : {}),
      };
    }
    default:
      break;
  }
  if (prompt.tool_name !== PLAN_TOOL) return { type: "permission" };
  let plan = "";
  let allowed: PlanAllowance[] = [];
  for (let i = before.length - 1; i >= 0; i -= 1) {
    const block = before[i]!;
    if (block.kind === "tool" && block.id === prompt.tool_use_id) allowed = allowances(block.input);
    if (block.kind === "assistant" && block.text?.trim()) {
      plan = block.text;
      break;
    }
    if (block.kind === "user") break;
  }
  return { type: "plan", plan, allowed };
}

function allowances(input: unknown): PlanAllowance[] {
  const list = (input as { allowed_prompts?: unknown } | null)?.allowed_prompts;
  if (!Array.isArray(list)) return [];
  return list
    .filter((a): a is { tool: unknown; prompt: unknown } => a != null && typeof a === "object")
    .map((a) => ({ tool: String(a.tool ?? ""), prompt: String(a.prompt ?? "") }))
    .filter((a) => a.prompt);
}

const record = (v: unknown): Record<string, unknown> => (v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : {});
const num = (v: unknown): number | undefined => (typeof v === "number" && Number.isFinite(v) ? v : undefined);

/** `requested_schema`（一个 object，属性都是原始类型，MCP elicitation）读成字段，按它列出的次序。 */
export function elicitationFields(schema: unknown): ElicitField[] {
  const root = record(schema);
  const required = new Set(Array.isArray(root.required) ? root.required.map(String) : []);
  return Object.entries(record(root.properties)).map(([name, raw]) => {
    const prop = record(raw);
    const field: ElicitField = {
      name,
      title: typeof prop.title === "string" && prop.title ? prop.title : name,
      required: required.has(name),
      kind: "unsupported",
    };
    if (typeof prop.description === "string" && prop.description) field.description = prop.description;
    const choices = choicesOf(prop);
    if (choices) {
      field.kind = "choice";
      field.choices = choices;
    } else if (prop.type === "string") {
      field.kind = "text";
      if (typeof prop.format === "string") field.format = prop.format;
      field.minLength = num(prop.minLength);
      field.maxLength = num(prop.maxLength);
    } else if (prop.type === "number" || prop.type === "integer") {
      field.kind = prop.type;
      field.minimum = num(prop.minimum);
      field.maximum = num(prop.maximum);
    } else if (prop.type === "boolean") {
      field.kind = "boolean";
    }
    const given = prop.default;
    if (typeof given === "string" || typeof given === "number" || typeof given === "boolean") field.default = given;
    for (const k of ["minLength", "maxLength", "minimum", "maximum"] as const) if (field[k] === undefined) delete field[k];
    return field;
  });
}

/** 选一个的字段：`enum`（可带 `enumNames`），或 `oneOf` 里一个个 `{const, title}`。 */
function choicesOf(prop: Record<string, unknown>): AskChoice[] | null {
  if (prop.type !== "string" && prop.type !== undefined) return null;
  if (Array.isArray(prop.enum)) {
    const names = Array.isArray(prop.enumNames) ? prop.enumNames : [];
    return prop.enum.map((v, i) => ({ key: String(v), label: String(names[i] ?? v) }));
  }
  if (Array.isArray(prop.oneOf) && prop.oneOf.every((o) => record(o).const !== undefined)) {
    return prop.oneOf.map((o) => {
      const r = record(o);
      return { key: String(r.const), label: String(r.title ?? r.const) };
    });
  }
  return null;
}

/** 一个字段为什么交不了。 */
export type ElicitProblem = "required" | "number" | "integer" | "range" | "length" | "unsupported";

/** 表单里填的：文字字段是原文，是否字段是勾没勾。 */
export type ElicitValues = Record<string, string | boolean | undefined>;

/** 填的东西按字段读成交回去的 `content`；有交不了的就说哪几个、为什么。没填的选填字段不交。 */
export function elicitationContent(
  fields: readonly ElicitField[],
  values: ElicitValues,
): { content: Record<string, unknown> } | { problems: Record<string, ElicitProblem> } {
  const content: Record<string, unknown> = {};
  const problems: Record<string, ElicitProblem> = {};
  for (const field of fields) {
    const raw = values[field.name] ?? field.default;
    if (field.kind === "boolean") {
      content[field.name] = raw === true;
      continue;
    }
    const text = raw == null || raw === false ? "" : String(raw);
    if (field.kind === "unsupported") {
      if (field.required) problems[field.name] = "unsupported";
      continue;
    }
    if (text.trim() === "") {
      if (field.required) problems[field.name] = "required";
      continue;
    }
    if (field.kind === "number" || field.kind === "integer") {
      const n = Number(text.trim());
      if (!Number.isFinite(n)) problems[field.name] = "number";
      else if (field.kind === "integer" && !Number.isInteger(n)) problems[field.name] = "integer";
      else if ((field.minimum != null && n < field.minimum) || (field.maximum != null && n > field.maximum)) problems[field.name] = "range";
      else content[field.name] = n;
      continue;
    }
    if ((field.minLength != null && text.length < field.minLength) || (field.maxLength != null && text.length > field.maxLength)) {
      problems[field.name] = "length";
      continue;
    }
    content[field.name] = text;
  }
  return Object.keys(problems).length ? { problems } : { content };
}

/**
 * 卡片的结局。权限与计划：允许、拒绝；提问：答了（`replied`）、不答（`declined`）；MCP 询问：接受、拒绝、不选。
 * 另外三种谁的都一样：超时（引擎超时即按拒绝或「没答」处理）、引擎撤回、别处已答（还不知道答的是什么）。
 */
export type AskAnswer = "allow" | "deny" | "timeout" | "withdrawn" | "answered" | "replied" | "accepted" | "declined" | "cancelled";

/** 一个回答在卡片上的样子：结局，和要一起显示的话（提问的答案、要求修改的意见）。 */
export function answerOf(
  form: AskForm,
  given: { decision?: Decision | null; answer?: AskReply | null },
): { answered: AskAnswer; reply?: string } {
  const { decision, answer } = given;
  if (decision) {
    if (decision.type === "deny") {
      // 权限卡的拒绝带的是界面自己写的理由；计划的是人写的修改意见。
      return form.type === "plan" && decision.reason ? { answered: "deny", reply: decision.reason } : { answered: "deny" };
    }
    return { answered: "allow" };
  }
  switch (answer?.type) {
    case "answer": {
      const chosen = form.type === "question" ? form.options.find((o) => o.key === answer.text) : undefined;
      return { answered: "replied", reply: chosen ? chosen.label : answer.text };
    }
    case "decline":
      return { answered: "declined" };
    case "accept":
      return { answered: "accepted" };
    case "cancel":
      return { answered: "cancelled" };
    default:
      return { answered: "answered" };
  }
}

/** 中枢说的「这个提问被这样答了」读成卡片的结局。 */
export function answeredBy(form: AskForm, event: AskAnswered): { answered: AskAnswer; reply?: string } {
  return answerOf(form, { decision: (event.decision as Decision | undefined) ?? null, answer: event.answer ?? null });
}
