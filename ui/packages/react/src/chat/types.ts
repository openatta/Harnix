/**
 * ChatPanel 的配置（界面设计 §4.4："类目会话、条目会话、工位、集成页、⌘K 用的是同一个组件，只是配置不同"）。
 *
 * 通用：不含任何 IPD 概念。领域的东西（提议卡、编号的样子、@ 搜什么、点编号去哪）由应用经这些钩子给。
 */

import { createContext, useContext, type ReactNode } from "react";
import type { Block, BlockOf } from "@harnix/core/state";
import type { ToolRowContribution, UserRowContribution } from "../components/blocks";
import { TOOL_NAMES } from "./format";

/** 回答者：头像是一个字（或两个字母），按角色上色。 */
export interface ChatRole {
  name: string;
  /** 头像里的字 */
  avatar: string;
  /** 头像底色（CSS 颜色或 var(--…)）；缺省 --hx-av */
  color?: string;
}

export type ChatTone = "run" | "ok" | "warn" | "bad" | "idle";

/** 抬头：一行上下文（← 面包屑 角色 状态 模型 … 操作），下面是能做什么、附加行、已选条目。 */
export interface ChatHeaderConfig {
  /** 面包屑："特性 › SR-002" */
  crumb?: ReactNode;
  /** 有它就显示"←"：回到上一层（类目） */
  onBack?: () => void;
  backLabel?: string;
  /** 回答者；缺省用 ChatPanel 的 assistant */
  role?: ChatRole | false;
  /** true：显示会话名（通用会话壳用） */
  title?: boolean;
  /** 上下文能做什么（小字） */
  scope?: ReactNode;
  /** 状态标签：运行中、等待你、已停…… */
  status?: { text: string; tone?: ChatTone };
  /** 模型（只读） */
  model?: string;
  /** 其余放在第一行的内容 */
  meta?: ReactNode;
  /** 第一行右侧的按钮 */
  actions?: ReactNode;
  /** scope 下面的整行内容（例如工位的线内运行条） */
  extra?: ReactNode;
  /** 用量胶囊（data-testid=conv-usage） */
  usage?: boolean;
  /** 连接状态 */
  connection?: boolean;
  /** 抬头上的 data-* 属性（测试与应用定位用），键不带 data- 前缀 */
  data?: Record<string, string | undefined>;
}

export interface QuickReply {
  /** 胶囊上的字 */
  label: string;
  /** 发出去的话；缺省就是 label */
  text?: string;
}

/** @ 菜单的一项 */
export interface MentionItem {
  id: string;
  /** 标题（菜单第二行） */
  label?: string;
  /** 选中后插入的文字；缺省 `${id} ` */
  insert?: string;
}

/** 斜杠命令（缺省取 harnix.hello 的 commands：引擎真支持的那些，原样作为消息发出） */
export interface ChatCommand {
  name: string;
  description?: string;
  /** 给了就选中即执行：不插入文字、不发送，输入框里已打的 `/xxx` 清掉 */
  run?: () => void | Promise<void>;
}

/**
 * 功能开关。原型输入框上的元素缺省都画出来；后端还不支持的（附件、语音、换模型、深入思考）画成不可用、
 * 弹层里写明"后端还不支持"，不假装能用。
 */
export interface ChatFeatures {
  /** "/" 命令菜单；缺省开（没有命令就不弹） */
  slash?: boolean;
  /** "@" 引用菜单；缺省：给了 mention 就开 */
  mention?: boolean;
  /** 悬停复制；缺省开 */
  copy?: boolean;
  /**
   * 上下文环（原型 ctxBtn）：缺省转录开、会话关。用量来自 turn_complete 的 usage（max 不知道就只显示 token 数），
   * 或应用给的 ratio；menu 是点开后的弹层里"压缩""清空"之类的行
   */
  contextRing?: boolean | { max?: number; ratio?: number; menu?: ReactNode; testId?: string; note?: ReactNode };
  /** ⚙ 选项：转录里"收起所有 diff"（纯前端，真能用），其余项要后端、不可用。缺省开 */
  options?: boolean;
  /** ＋ 添加：文件与图片（给了 upload）、引用条目（给了 mention）、命令；截图要后端，不可用。缺省开 */
  attachments?: boolean;
  /** 语音输入：要后端（转写），画成不可用的按钮。缺省开 */
  voice?: boolean;
  /** 模型（底栏右侧）：显示当前模型；给了 onModel 才能换。缺省开 */
  modelPicker?: boolean;
}

/** 流里任何一块的贡献（`flow.block`）：后注册的先匹配；没人认领时内建的画，内建不认识的不画。 */
export interface FlowBlockContribution {
  id: string;
  match(block: Block): boolean;
  render(block: Block): ReactNode;
}

export interface ChatRenderers {
  /** 流里的块（先于下面的一切） */
  flowBlocks?: readonly FlowBlockContribution[];
  /** 工具行的注册表（后注册的先匹配，没人认领用内建的） */
  toolRows?: readonly ToolRowContribution[];
  /** 人的消息的注册表（后注册的先匹配，没人认领用气泡） */
  userRows?: readonly UserRowContribution[];
  /** 助手一段话后面的附加内容（例如疑点卡、动作按钮） */
  assistantExtras?: (block: BlockOf<"assistant">) => ReactNode;
  /** 工具行"⎿"后面那一句（领域工具的结果怎么说最明白）；返回空就用内建的摘要 */
  toolSummary?: (block: BlockOf<"tool">) => string | null | undefined;
}

/** 正文里的编号：匹配到的变成 chip；给了 onOpen 就可以点 */
export interface ChatIds {
  /** 带 g 标志的正则，例如 /\b(?:SR|SL|IR|P|LR)-\d+(?:-\d+)?\b/g */
  pattern: RegExp;
  onOpen?: (id: string) => void;
  /** 哪些编号可以点（缺省：给了 onOpen 就都可以） */
  canOpen?: (id: string) => boolean;
}

export type ChatVariant = "conversation" | "transcript";

/** 组件内部共享的配置（工具行、编号、diff 开合） */
export interface ChatContextValue {
  variant: ChatVariant;
  toolNames: Record<string, string>;
  ids?: ChatIds;
  /** "收起所有 diff" */
  diffsCollapsed: boolean;
  /** 每次切换"收起所有 diff"加一：各行自己点过的开合作废 */
  diffGeneration: number;
  toolSummary?: ChatRenderers["toolSummary"];
}

export const ChatContext = createContext<ChatContextValue>({
  variant: "conversation",
  toolNames: TOOL_NAMES,
  diffsCollapsed: false,
  diffGeneration: 0,
});

export function useChat(): ChatContextValue {
  return useContext(ChatContext);
}
