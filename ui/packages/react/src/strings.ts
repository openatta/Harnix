/**
 * The conversation components' text as one typed table: what `<HarnixUiProvider strings>` takes,
 * whole or in part, and what `useStrings()` gives a component.
 *
 * The words themselves are in the dictionaries (`i18n/*.ts`, the `chat.*` keys), with the rest of
 * the interface's: one place to write them, one test keeping the two languages' keys and
 * placeholders the same, one term list (`tests/style-lint.mjs`). Both live in `@harnix/react`, so
 * the components need nothing of the shell to speak either language. A table is made for a
 * language at a time (`uiStrings`), and a sentence that takes values is a whole sentence per case
 * in the dictionaries, never pieces put together here.
 */

import { translate, type Locale } from "./i18n/index";

export interface HarnixUiStrings {
  untitled: string;
  connOpen: string;
  connConnecting: string;
  connClosed: string;
  connIncompatible: string;
  loading: string;
  openFailed: (error: string) => string;
  replayTruncated: string;
  olderHidden: (count: number) => string;
  empty: string;
  inputPlaceholder: string;
  inputPlaceholderRunning: string;
  send: string;
  interrupt: string;
  interruptFailed: (error: string) => string;
  sendFailed: (error: string) => string;
  running: string;
  runningElsewhere: string;
  queued: (index: number) => string;
  queueRemove: string;
  queueEdit: string;
  queueSave: string;
  queueCancel: string;
  removeAttachment: string;
  uploading: string;
  interrupted: string;
  turnFailed: (error: string) => string;
  stopReason: (reason: string) => string;
  /** 人的话被拒收（ipdrun-13）：`reason` 是理由；`text` 是被拒的那句，只在它不在流里时给（steer）。 */
  messageRefused?: (reason: string, text?: string) => string;
  tokens: (count: number) => string;
  cached: (count: number) => string;
  toolInput: string;
  toolResult: string;
  toolError: string;
  toolRunning: string;
  /** A call a permission prompt is open for. */
  toolAwaiting: string;
  toolFailed: string;
  result: string;
  image: string;
  injectedContext: string;
  /** 一次压缩的一行：知道前后几条时说压了几条，知道省了多少时也说。 */
  compacted: (facts: { before?: number; after?: number; saved?: number }) => string;
  thinking: string;
  subagent: string;
  subagentDone: string;
  permRequest: (tool: string) => string;
  /** `decision` is one of the `permDecision*` texts of the same table. */
  permAnswered: (decision: string, tool: string) => string;
  permAllowOnce: string;
  permAllowSession: string;
  permAllowLocal: string;
  permAllowLocalHint: string;
  permDeny: string;
  permDecisionAllow: string;
  permDecisionDeny: string;
  permDecisionTimeout: string;
  permCountdown: (seconds: number) => string;
  answerFailed: (error: string) => string;
  sessions: string;
  newSession: string;
  noSessions: string;
  dismiss: string;
  /* 会话组件（ChatPanel） */
  assistant: string;
  back: string;
  selected: string;
  clearSelection: string;
  replying: string;
  thinkingLive: string;
  thoughtFor: (seconds: number) => string;
  working: string;
  escToInterrupt: string;
  /** The first Escape of a running turn: the second interrupts it. */
  escAgainToInterrupt: string;
  copy: string;
  copied: string;
  queuedInFlow: string;
  /** 交给正在跑的一轮、还没进对话（IPDrun N-108） */
  steerPending: string;
  /** 在这一轮的步骤边界进了对话 */
  steerDelivered: string;
  stop: string;
  stopHint: string;
  sendHint: string;
  readOnly: string;
  commands: string;
  noCommands: string;
  mentions: string;
  noMentions: string;
  toolInterrupted: string;
  toolDone: string;
  readLines: (n: number) => string;
  editLines: (added: number, removed: number) => string;
  wroteLines: (n: number) => string;
  results: (n: number) => string;
  expand: string;
  collapse: string;
  options: string;
  collapseDiffs: string;
  collapseDiffsHint: string;
  attach: string;
  voice: string;
  model: string;
  /** The model button's label for screen readers: "Model: <name>". */
  modelOf: (model: string) => string;
  /** The model button when no model is configured. */
  modelUnset: string;
  modelUnsetHint: string;
  needsBackend: string;
  contextUsage: (tokens: number, max: number | null) => string;
  retry: string;
  plusMenu: string;
  uploadFile: string;
  uploadImage: string;
  insertCommand: string;
  insertCommandHint: string;
  attachFootReady: string;
  modelCustom: string;
  modelFootSwitch: string;
  screenshot: string;
  mentionItem: string;
  mentionItemHint: string;
  attachFoot: string;
  sessionOptions: string;
  stationOptions: string;
  deepThinking: string;
  deepThinkingHint: string;
  webSearch: string;
  autoCompact: string;
  modelFoot: string;
  context: string;
  /** 上下文弹层的小字：自动压缩从哪里开始（百分比） */
  autoCompactAt: (percent: number) => string;
  interruptStep: string;
}

/** The table in one language, read from the dictionaries. */
export function uiStrings(locale: Locale): HarnixUiStrings {
  const s = (key: string, vars?: Record<string, unknown>) => translate(locale, `chat.${key}`, vars);
  const allow = s("permDecisionAllow");
  const deny = s("permDecisionDeny");
  return {
    untitled: s("untitled"),
    connOpen: s("connOpen"),
    connConnecting: s("connConnecting"),
    connClosed: s("connClosed"),
    connIncompatible: s("connIncompatible"),
    loading: s("loading"),
    replayTruncated: s("replayTruncated"),
    empty: s("empty"),
    inputPlaceholder: s("inputPlaceholder"),
    inputPlaceholderRunning: s("inputPlaceholderRunning"),
    send: s("send"),
    interrupt: s("interrupt"),
    running: s("running"),
    runningElsewhere: s("runningElsewhere"),
    queueRemove: s("queueRemove"),
    queueEdit: s("queueEdit"),
    queueSave: s("queueSave"),
    queueCancel: s("queueCancel"),
    removeAttachment: s("removeAttachment"),
    uploading: s("uploading"),
    interrupted: s("interrupted"),
    toolInput: s("toolInput"),
    toolResult: s("toolResult"),
    toolError: s("toolError"),
    toolRunning: s("toolRunning"),
    toolAwaiting: s("toolAwaiting"),
    toolFailed: s("toolFailed"),
    result: s("result"),
    image: s("image"),
    injectedContext: s("injectedContext"),
    thinking: s("thinking"),
    subagent: s("subagent"),
    subagentDone: s("subagentDone"),
    permAllowOnce: s("permAllowOnce"),
    permAllowSession: s("permAllowSession"),
    permAllowLocal: s("permAllowLocal"),
    permAllowLocalHint: s("permAllowLocalHint"),
    permDeny: s("permDeny"),
    permDecisionAllow: s("permDecisionAllow"),
    permDecisionDeny: s("permDecisionDeny"),
    permDecisionTimeout: s("permDecisionTimeout"),
    sessions: s("sessions"),
    newSession: s("newSession"),
    noSessions: s("noSessions"),
    dismiss: s("dismiss"),
    assistant: s("assistant"),
    back: s("back"),
    selected: s("selected"),
    clearSelection: s("clearSelection"),
    replying: s("replying"),
    thinkingLive: s("thinkingLive"),
    working: s("working"),
    escToInterrupt: s("escToInterrupt"),
    escAgainToInterrupt: s("escAgainToInterrupt"),
    copy: s("copy"),
    copied: s("copied"),
    queuedInFlow: s("queuedInFlow"),
    steerPending: s("steerPending"),
    steerDelivered: s("steerDelivered"),
    stop: s("stop"),
    stopHint: s("stopHint"),
    sendHint: s("sendHint"),
    readOnly: s("readOnly"),
    commands: s("commands"),
    noCommands: s("noCommands"),
    mentions: s("mentions"),
    noMentions: s("noMentions"),
    toolInterrupted: s("toolInterrupted"),
    toolDone: s("toolDone"),
    expand: s("expand"),
    collapse: s("collapse"),
    options: s("options"),
    collapseDiffs: s("collapseDiffs"),
    collapseDiffsHint: s("collapseDiffsHint"),
    attach: s("attach"),
    voice: s("voice"),
    model: s("model"),
    modelUnset: s("modelUnset"),
    modelUnsetHint: s("modelUnsetHint"),
    needsBackend: s("needsBackend"),
    retry: s("retry"),
    plusMenu: s("plusMenu"),
    uploadFile: s("uploadFile"),
    uploadImage: s("uploadImage"),
    insertCommand: s("insertCommand"),
    insertCommandHint: s("insertCommandHint"),
    attachFootReady: s("attachFootReady"),
    modelCustom: s("modelCustom"),
    modelFootSwitch: s("modelFootSwitch"),
    screenshot: s("screenshot"),
    mentionItem: s("mentionItem"),
    mentionItemHint: s("mentionItemHint"),
    attachFoot: s("attachFoot"),
    sessionOptions: s("sessionOptions"),
    stationOptions: s("stationOptions"),
    deepThinking: s("deepThinking"),
    deepThinkingHint: s("deepThinkingHint"),
    webSearch: s("webSearch"),
    autoCompact: s("autoCompact"),
    modelFoot: s("modelFoot"),
    context: s("context"),
    interruptStep: s("interruptStep"),
    openFailed: (error) => s("openFailed", { error }),
    olderHidden: (count) => s("olderHidden", { count }),
    interruptFailed: (error) => s("interruptFailed", { error }),
    sendFailed: (error) => s("sendFailed", { error }),
    queued: (index) => s("queued", { index }),
    turnFailed: (error) => s("turnFailed", { error }),
    stopReason: (reason) => s("stopReason", { reason }),
    messageRefused: (reason, text) =>
      text ? (reason ? s("refusedTextBecause", { text, reason }) : s("refusedText", { text })) : reason ? s("refusedBecause", { reason }) : s("refused"),
    tokens: (count) => s("tokens", { count: count.toLocaleString() }),
    cached: (count) => s("cached", { count: count.toLocaleString() }),
    compacted: ({ before, after, saved }) =>
      before != null && after != null ? s(saved ? "compactedSaved" : "compactedCounts", { before, after, saved }) : s("compacted"),
    permRequest: (tool) => s("permRequest", { tool }),
    permAnswered: (decision, tool) => (decision === allow ? s("permAnsweredAllow", { tool }) : decision === deny ? s("permAnsweredDeny", { tool }) : s("permAnsweredTimeout", { tool })),
    permCountdown: (seconds) => s("permCountdown", { seconds }),
    answerFailed: (error) => s("answerFailed", { error }),
    thoughtFor: (seconds) => s("thoughtFor", { seconds }),
    readLines: (n) => s("readLines", { n }),
    editLines: (added, removed) => s("editLines", { added, removed }),
    wroteLines: (n) => s("wroteLines", { n }),
    results: (n) => s("results", { n }),
    contextUsage: (tokens, max) =>
      max
        ? s("contextUsageOf", { tokens: tokens.toLocaleString(), max: max.toLocaleString(), percent: Math.round((tokens / max) * 100) })
        : s("contextUsage", { tokens: tokens.toLocaleString() }),
    autoCompactAt: (percent) => s("autoCompactAt", { percent }),
    modelOf: (model) => s("modelOf", { model }),
  };
}

export const zhCN: HarnixUiStrings = uiStrings("zh-CN");
export const en: HarnixUiStrings = uiStrings("en");
