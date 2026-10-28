# helm-01：模块不知道是哪台设备回答了某个工具的权限卡

类型：新能力｜优先级：中

## 需求

模块的原生工具执行写操作前，引擎会发权限卡（`prompt` 事件，带 `tool_use_id`），任何一个观看者都能回答（`session.respondToPrompt`）。
模块要在自己的审计里记下「谁确认了这次写」，但：

- 引擎只收 `{prompt_id, decision}`，不知道设备；设备与准入归中枢（`crates/hub/src/lib.rs` 的 `dispatch`）。
- 工具执行时拿到的 `ToolContext` 有 `session_id`、`tool_use_id`，没有回答者。

所以模块只能把「人确认过」与「会话的权限模式直接放行」记成一样，审计说不清是谁。

## 建议

```jsonc
// 外壳回答权限卡时多带一个可选字段（中枢转给引擎前去掉）
session.respondToPrompt { instance, session_id, prompt_id, decision, tool_use_id? }

// 中枢方法，只许模块与内核
harnix.prompts.answerer { session_id, tool_use_id } -> { answerer: "device:<id>" | null }
```

- 中枢在设备回答、且带了 `tool_use_id` 时记下 `(session_id, tool_use_id) → device:<id>`；同一张卡先答者为准。
- 读一次即忘；最多留最近 1024 条，没人读的不会一直占着。
- `null`：没有设备回答（权限模式直接放行）、已读过、或已被挤掉。
- 设备调用答 `REFUSED`。

## 兼容性

- `tool_use_id` 可选：不带的客户端照旧，引擎看不到这个字段。
- 新方法只加不改；不用它的应用没有变化。协议版本不必升。

## 验收

- `tests/live/api/prompt-answerer.spec.ts`：系统会话放录像 `runs-a-while`，等到权限卡，设备带 `tool_use_id` 回答；
  `app.probe.call` 以模块身份读到 `device:…`，再读是 `null`；设备直接调用答 `REFUSED`。

## 依赖

无（Agix 不改）。

## 来源

Helm 的 Agent 写操作（服务运维、流程动词）要在审计里另写一行「谁确认的」，以 `intent_id` 与操作本身的审计相连。

## 实现记录

- 中枢：`Hub.answerers`（有界队列，`ANSWERERS_KEPT = 1024`），`session.respondToPrompt` 经 `dispatch` 时取下 `tool_use_id` 记录；
  `harnix.prompts.answerer` 加进 `HUB_METHODS`。
- 外壳：`SessionController.answer` 带上卡上的 `tool_use_id`；`frames.ts` 的参数类型加可选字段。
