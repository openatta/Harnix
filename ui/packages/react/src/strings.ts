/**
 * 界面文案。Harnix 的规矩是"只用 key，不写死文案"（概念与架构 §5.3）；这里是同一件事的
 * React 版本：组件只读这张表，应用经 `<HarnixUiProvider strings>` 换掉整张或其中几项。
 */

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
  removeAttachment: string;
  uploading: string;
  interrupted: string;
  turnFailed: (error: string) => string;
  stopReason: (reason: string) => string;
  tokens: (count: number) => string;
  cached: (count: number) => string;
  toolInput: string;
  toolResult: string;
  toolError: string;
  toolRunning: string;
  toolFailed: string;
  result: string;
  image: string;
  injectedContext: string;
  thinking: string;
  subagent: string;
  subagentDone: string;
  permRequest: (tool: string) => string;
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

export const zhCN: HarnixUiStrings = {
  untitled: "未命名会话",
  connOpen: "已连接",
  connConnecting: "连接中…",
  connClosed: "实时更新已断开",
  connIncompatible: "版本不兼容",
  loading: "正在打开会话…",
  openFailed: (error) => `打开会话失败：${error}`,
  replayTruncated: "这一轮太长，缓冲区只保留了后面一段；完整内容在本轮结束后从历史里补齐。",
  olderHidden: (count) => `更早的 ${count} 条消息未显示`,
  empty: "说点什么开始吧。",
  inputPlaceholder: "输入消息，Enter 发送，Shift+Enter 换行",
  inputPlaceholderRunning: "插话：运行中会排队，这一轮结束后送达",
  send: "发送",
  interrupt: "中断",
  interruptFailed: (error) => `中断失败：${error}`,
  sendFailed: (error) => `发送失败：${error}`,
  running: "正在回复…",
  runningElsewhere: "这个会话正在别处运行",
  queued: (index) => `排队 ${index}`,
  queueRemove: "撤回",
  removeAttachment: "去掉这个附件",
  uploading: "正在上传…",
  interrupted: "已中断",
  turnFailed: (error) => `这一轮失败了：${error}`,
  stopReason: (reason) => `停止原因：${reason}`,
  tokens: (count) => `${count.toLocaleString()} tokens`,
  cached: (count) => `缓存 ${count.toLocaleString()}`,
  toolInput: "输入",
  toolResult: "结果",
  toolError: "错误",
  toolRunning: "运行中…",
  toolFailed: "失败",
  result: "结果",
  image: "[图片]",
  injectedContext: "注入的上下文",
  thinking: "思考",
  subagent: "子 agent",
  subagentDone: "完成",
  permRequest: (tool) => `请求使用 ${tool}`,
  permAnswered: (decision, tool) => `${decision}：${tool}`,
  permAllowOnce: "允许一次",
  permAllowSession: "本会话内允许",
  permAllowLocal: "总是允许",
  permAllowLocalHint: "写进本项目的本地设置",
  permDeny: "拒绝",
  permDecisionAllow: "已允许",
  permDecisionDeny: "已拒绝",
  permDecisionTimeout: "超时未答，已按拒绝处理",
  permCountdown: (seconds) => `${seconds} 秒后按拒绝处理`,
  answerFailed: (error) => `回答失败：${error}`,
  sessions: "会话",
  newSession: "新会话",
  noSessions: "还没有会话",
  dismiss: "关闭",
  assistant: "助手",
  back: "返回",
  selected: "已选",
  clearSelection: "清除",
  replying: "正在回复",
  thinkingLive: "正在思考",
  thoughtFor: (seconds) => `思考了 ${seconds} 秒`,
  working: "工作中…",
  escToInterrupt: "Esc 中断",
  copy: "复制",
  copied: "已复制",
  queuedInFlow: "排队中 · 当前步骤结束后送达",
  steerPending: "待送达 · 当前步骤结束后送进这一轮",
  steerDelivered: "已送达 · 这一轮的下一步",
  stop: "停止",
  stopHint: "停止（Esc）",
  sendHint: "发送（Enter）",
  readOnly: "这个会话是只读的。",
  commands: "命令",
  noCommands: "没有匹配的命令",
  mentions: "引用条目",
  noMentions: "没有匹配的条目",
  toolInterrupted: "⏸ 已中断",
  toolDone: "完成",
  readLines: (n) => `读取 ${n} 行`,
  editLines: (added, removed) => `增加 ${added} 行，删除 ${removed} 行`,
  wroteLines: (n) => `写入 ${n} 行`,
  results: (n) => `${n} 条结果`,
  expand: "展开",
  collapse: "收起",
  options: "选项",
  collapseDiffs: "收起所有 diff",
  collapseDiffsHint: "转录更紧凑，点工具行可展开",
  attach: "添加文件或引用",
  voice: "语音输入",
  model: "模型",
  needsBackend: "后端还不支持",
  contextUsage: (tokens, max) =>
    max ? `上下文 ${tokens.toLocaleString()} / ${max.toLocaleString()} tokens（${Math.round((tokens / max) * 100)}%）` : `上下文 ${tokens.toLocaleString()} tokens`,
  retry: "重新生成（后端还不支持）",
  plusMenu: "添加到这条消息",
  uploadFile: "上传文件",
  uploadImage: "添加图片",
  insertCommand: "命令",
  insertCommandHint: "输入 / 也可以",
  attachFootReady: "也可以把文件拖进来，或直接粘贴",
  modelCustom: "其他模型名，回车切换",
  modelFootSwitch: "只对这个会话生效",
  screenshot: "截取屏幕",
  mentionItem: "引用条目",
  mentionItemHint: "输入 @ 也可以",
  attachFoot: "附件、截图与语音需要后端支持，暂不可用",
  sessionOptions: "这个会话",
  stationOptions: "工位选项",
  deepThinking: "深入思考",
  deepThinkingHint: "回答前先推理，慢一些，适合拆分与评审",
  webSearch: "联网搜索",
  autoCompact: "自动压缩上下文",
  modelFoot: "换模型需要后端支持（按会话换模型），暂不可用。",
  context: "上下文",
  autoCompactAt: (percent) => `超过 ${percent}% 自动压缩`,
  interruptStep: "中断当前步骤（Esc）",
};

export const en: HarnixUiStrings = {
  untitled: "Untitled session",
  connOpen: "Connected",
  connConnecting: "Connecting…",
  connClosed: "Live updates disconnected",
  connIncompatible: "Incompatible version",
  loading: "Opening session…",
  openFailed: (error) => `Could not open the session: ${error}`,
  replayTruncated: "This turn is long; the buffer kept only its tail. The rest comes from history when it settles.",
  olderHidden: (count) => `${count} earlier messages are not shown`,
  empty: "Say something to start.",
  inputPlaceholder: "Message — Enter to send, Shift+Enter for a newline",
  inputPlaceholderRunning: "Interject: queued while running, delivered when this turn ends",
  send: "Send",
  interrupt: "Interrupt",
  interruptFailed: (error) => `Interrupt failed: ${error}`,
  sendFailed: (error) => `Send failed: ${error}`,
  running: "Replying…",
  runningElsewhere: "This session is running elsewhere",
  queued: (index) => `Queued ${index}`,
  queueRemove: "Withdraw",
  removeAttachment: "Remove this attachment",
  uploading: "Uploading…",
  interrupted: "Interrupted",
  turnFailed: (error) => `The turn failed: ${error}`,
  stopReason: (reason) => `Stopped: ${reason}`,
  tokens: (count) => `${count.toLocaleString()} tokens`,
  cached: (count) => `${count.toLocaleString()} cached`,
  toolInput: "Input",
  toolResult: "Result",
  toolError: "Error",
  toolRunning: "running…",
  toolFailed: "failed",
  result: "Result",
  image: "[image]",
  injectedContext: "Injected context",
  thinking: "Thinking",
  subagent: "Sub-agent",
  subagentDone: "done",
  permRequest: (tool) => `Wants to use ${tool}`,
  permAnswered: (decision, tool) => `${decision}: ${tool}`,
  permAllowOnce: "Allow once",
  permAllowSession: "Allow for this session",
  permAllowLocal: "Always allow",
  permAllowLocalHint: "Written to this project's local settings",
  permDeny: "Deny",
  permDecisionAllow: "Allowed",
  permDecisionDeny: "Denied",
  permDecisionTimeout: "No answer in time; denied",
  permCountdown: (seconds) => `Denied in ${seconds}s`,
  answerFailed: (error) => `Answer failed: ${error}`,
  sessions: "Sessions",
  newSession: "New session",
  noSessions: "No sessions yet",
  dismiss: "Dismiss",
  assistant: "Assistant",
  back: "Back",
  selected: "Selected",
  clearSelection: "Clear",
  replying: "Replying",
  thinkingLive: "Thinking",
  thoughtFor: (seconds) => `Thought for ${seconds}s`,
  working: "Working…",
  escToInterrupt: "Esc to interrupt",
  copy: "Copy",
  copied: "Copied",
  queuedInFlow: "Queued · delivered when this step ends",
  steerPending: "Pending · goes into this turn when the current step ends",
  steerDelivered: "Delivered · next step of this turn",
  stop: "Stop",
  stopHint: "Stop (Esc)",
  sendHint: "Send (Enter)",
  readOnly: "This conversation is read-only.",
  commands: "Commands",
  noCommands: "No matching commands",
  mentions: "Reference an item",
  noMentions: "No matching items",
  toolInterrupted: "Interrupted",
  toolDone: "Done",
  readLines: (n) => `Read ${n} lines`,
  editLines: (added, removed) => `Added ${added} lines, removed ${removed}`,
  wroteLines: (n) => `Wrote ${n} lines`,
  results: (n) => `${n} results`,
  expand: "Expand",
  collapse: "Collapse",
  options: "Options",
  collapseDiffs: "Collapse all diffs",
  collapseDiffsHint: "A tighter transcript; click a tool row to expand",
  attach: "Add files or references",
  voice: "Voice input",
  model: "Model",
  needsBackend: "Not supported by the backend yet",
  contextUsage: (tokens, max) =>
    max ? `Context ${tokens.toLocaleString()} / ${max.toLocaleString()} tokens (${Math.round((tokens / max) * 100)}%)` : `Context ${tokens.toLocaleString()} tokens`,
  retry: "Regenerate (not supported by the backend yet)",
  plusMenu: "Add to this message",
  uploadFile: "Upload a file",
  uploadImage: "Add an image",
  insertCommand: "Command",
  insertCommandHint: "or type /",
  attachFootReady: "You can also drop or paste files here",
  modelCustom: "Another model name, Enter to switch",
  modelFootSwitch: "Applies to this conversation only",
  screenshot: "Capture the screen",
  mentionItem: "Reference an item",
  mentionItemHint: "or type @",
  attachFoot: "Attachments, screenshots and voice need backend support; not available yet",
  sessionOptions: "This conversation",
  stationOptions: "Station options",
  deepThinking: "Deep thinking",
  deepThinkingHint: "Reason before answering; slower",
  webSearch: "Web search",
  autoCompact: "Auto-compact context",
  modelFoot: "Switching models needs backend support; not available yet.",
  context: "Context",
  autoCompactAt: (percent) => `Auto-compacts above ${percent}%`,
  interruptStep: "Interrupt the current step (Esc)",
};
