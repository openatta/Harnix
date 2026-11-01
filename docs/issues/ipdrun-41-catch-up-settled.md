# ipdrun-41：tap 落后时，逐帧读的调用方拿不到缺口里的结算；结算也不带回答文本与工具调用

类型：新能力｜优先级：中

## 需求

ipdrun-17 之后 `harnix.turn_settled` 带 `outcome`，tap 满时结算帧另存（`crates/module/src/sessions.rs` 的 `Settled`）。还差两处：

- 另存的结算帧只有 `Tap::next_settled` 读得到。它跳过（吃掉）别的帧，和逐帧读 `Tap::next` 的循环不能混用；`Tap::catch_up` 只补回中枢缓冲里的事件，
  而缓冲在每一轮开头清空（`crates/hub/src/turns.rs` 的 `open_turn`），整轮都在缺口里的那几轮，逐帧读的调用方连 `turn_settled` 都收不到。
- `outcome` 只有 `stop_reason`、`usage`、`structured_output`、`message`。缺口里结束的一轮说了什么、调了哪些工具，只能读 `session.history` 去拼。

## 建议（只加不改）

- `Tap::catch_up_settled()`：同 `catch_up`，并把另存的结算帧放进 `next()` 读的队列——中枢缓冲里还有它那一轮的事件，就排在那些事件之后；
  否则排在回放之前（那几轮更早结束，缓冲只留最近一轮）。`catch_up`、`next_settled` 不变。
- `outcome` 加两项：`text`（这一轮的回答文本，`text_delta` 拼起来，最多 64 KiB，超了截尾并 `text_truncated: true`）、`tool_calls`
  （`[{id, name, is_error?}]`，`tool_use` 记下、`tool_result` 补 `is_error`）。中枢本来逐帧经手，记在会话状态里，不读转录。

## 兼容性

新方法、新字段都是可选的：不调 `catch_up_settled` 的模块行为不变；不认识 `text` / `tool_calls` 的客户端照旧。帧大小：`text` 有上限，
回答本来已经以 `text_delta` 发过一遍。协议版本不升。

## 验收

- `crates/module` 单测：容量 1 的 tap 不读、三轮过去（后两轮在缺口里，缓冲只留第三轮），`catch_up_settled` 之后 `next()` 依次读到
  t1、t2 的结算、t3 的事件、t3 的结算；`catch_up` 仍只补事件，结算留给 `next_settled`。
- `crates/hub` 单测：`outcome.text` 是各段 `text_delta` 拼起来（不含 thinking），`tool_calls` 带 `is_error`；长回答在字符边界截断并标 `text_truncated`。

## 依赖

ipdrun-17。

## 来源

IPDrun 的会话宿主逐帧读 tap；落后时缺口里的轮次按转录补文本、工具调用与结构化输出候选，整轮在缺口里的按转录结掉（HARNIX-WORKAROUND C1-#28）。

## 落地

落地提交：`7178d36`（main，2026-11-01，未推送；版本待下次发布）。与建议一致：`Tap::catch_up_settled()`；`outcome.text`（上限 64 KiB，`text_truncated`）、`outcome.tool_calls`（`[{id, name, is_error?}]`）；`@harnix/core` 的 `turn_settled.outcome` 类型补上这三项。
