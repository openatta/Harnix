/**
 * @harnix/react —— 会话的 React 组件（设计 §4.2、M4-3）。对话主干来自 IPDrun 的 nest-ui（D21，Apache-2.0）：
 * 对话、工具行、权限卡、markdown、diff、队列、草稿；语音、上下文环、快捷回复这类产品面留在 IPDrun。
 * 样式在 `@harnix/react/styles.css`，只用 CSS 变量（令牌），明暗两套。
 *
 * 会话组件只有一个：ChatPanel（对话 / 转录两种样子，其余靠配置）。SessionView 是它的一种配置。
 */

export { HarnixUiProvider, useConnection, useHello, useHarnixClient, useSession, useStrings, type HarnixUiProviderProps } from "./context";
export { ChatPanel, type ChatPanelProps } from "./chat/ChatPanel";
export { ChatComposer, type Attached, type ChatComposerProps, type ComposerApi } from "./chat/ChatComposer";
export { TranscriptList, type TranscriptListProps } from "./chat/TranscriptList";
export { readDraft, writeDraft } from "./chat/drafts";
export { Avatar, ComposerNote, DiffView, QueuedBubble, SegmentRow, TranscriptToolRow } from "./chat/messages";
export type { ContextRingConfig } from "./chat/ChatComposer";
export {
  type ChatCommand,
  type ChatFeatures,
  type ChatHeaderConfig,
  type ChatIds,
  type ChatRenderers,
  type ChatRole,
  type ChatTone,
  type ChatVariant,
  type MentionItem,
  type QuickReply,
} from "./chat/types";
export {
  TOOL_NAMES,
  chipifyHtml,
  clockTime,
  compactTokens,
  duration,
  editOf,
  numberedDiff,
  splitIds,
  toolArgs,
  toolDisplayName,
  toolSummary,
  type NumberedDiffLine,
} from "./chat/format";
export { SessionView, type SessionViewProps } from "./components/SessionView";
export { SessionList, type SessionListProps } from "./components/SessionList";
export { ConnectionBadge } from "./components/ConnectionBadge";
export { BuiltinToolRow, Disclosure, STATUS_GLYPH, ToolDetails, argumentSummary, clip, type ToolRowContribution, type UserRowContribution } from "./components/blocks";
export { lineDiff, hunks, type DiffLine } from "./diff";
export { markdown, escapeHtml } from "./markdown";
export { zhCN, en, type HarnixUiStrings } from "./strings";
export { Icon } from "./chat/icons";
export {
  POINTS,
  Registry,
  Contributed,
  useRegistry,
  type CommandCandidate,
  type Contributions,
  type DetailItem,
  type Point,
  type Refusal,
  type SidebarGroup,
  type SpaceContribution,
  type SpaceTab,
  tabsOf,
} from "./contrib";
export { t, useT, useLocale, setLocale, currentLocale, localeChoice, addStrings, DICTIONARIES, type Locale, type LocaleChoice, type Translate } from "./i18n/index";
export type { FlowBlockContribution } from "./chat/types";
export { NestProvider, useNest, type NestHost, type SessionRef as HostSessionRef } from "./host";
