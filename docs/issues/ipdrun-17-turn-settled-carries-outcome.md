# ipdrun-17：`harnix.turn_settled` 不带这一轮的结果

类型：新能力｜优先级：低

## 需求

服务端调用方（模块）驱动无头会话时，要知道一轮的结果：停止原因、用量、结构化输出、最后的回答。现在：

- `harnix.turn_settled` 的参数是 `{instance, session_id, turn_id, result}` 或 `{…, error}`（`crates/hub/src/turns.rs:321-332`），
  `result` 是引擎 `run_turn` 调用的应答，只有 `turn_id`、`delivery`、`name`、`api_calls` 这些；
- 停止原因、`usage`、`structured_output` 在引擎的 `turn_complete` 事件里（Agix `daemon/src/session_pool.rs:3507-3524`），
  经中枢以会话帧发出。

tap 是有界的（`TAP_CAPACITY = 512`，`crates/module/src/sessions.rs:284`）：一轮里帧多、或模块处理慢时 tap 收到 `Lagged`，
`turn_complete` 那一帧可能正好在缺口里。模块要么追赶（`catch_up`），要么读转录去补，各自写一套「这一轮到底怎么结束的」。

## 建议

中枢在结算时把它看到的这一轮 `turn_complete` 的要点附在 `turn_settled` 上（只加不改）：

```jsonc
// harnix.turn_settled
{
  "instance": "...", "session_id": "...", "turn_id": "...",
  "result": { ... },                 // 不变
  "outcome": {                       // 新增，可选：中枢看到了 turn_complete 才有
    "stop_reason": "end_turn",
    "usage": { ... },
    "structured_output": { ... },    // 有才有
    "message": "..."                 // 有才有（例如 hook 截停的理由）
  }
}
```

- 中枢本来就逐帧经手这一轮的事件，在会话状态里记下最后一个 `turn_complete` 的这几个字段即可，不必额外读转录。
- 最后的回答文本是否也带上（可能很长）交给维护者定；不带的话，至少在 SDK 里写明「结果以 `outcome` 为准，回答文本读 `session.history`」。
- 模块侧 `Tap` 可以加一个便捷方法 `next_settled() -> Settled`，只等结算帧。

## 兼容性

- 新字段可选：不认识它的客户端照旧。`result` 不变。协议版本不必升。
- 帧变大一点（`usage` 是一个小对象）；`structured_output` 可能较大，但本来就已经以 `turn_complete` 发过一次。
- Helm、hello-app 不改代码。Helm 的模块驱动无头会话时也是在 tap 里找 `turn_complete` 帧（`crates/helm-module/src/flow/agent.rs`），
  `Lagged` 时同样可能漏掉；有了 `outcome` / `next_settled()` 可以换过去。

## 验收

- `tests/live/api/turns.spec.ts`：一轮正常结束，`turn_settled.outcome.stop_reason == "end_turn"`，`usage` 与该轮 `turn_complete` 帧里的一致。
- 带结构化输出的一轮：`outcome.structured_output` 等于 `turn_complete` 里的。
- 被 hook 截停的一轮：`outcome.stop_reason == "stopped_by_hook"`、`message` 是 hook 的理由。
- tap 故意落后（容量填满）时，模块只看 `turn_settled` 也拿得到 `outcome`（`crates/testapp` 的用例）。

## 依赖

无（字段都来自 Agix 现有的 `turn_complete`）。

## 来源

IPDrun 的宿主在 tap 落后后按转录补每一轮的结果（`crates/agent/src/events.rs` 的 `absorb_history`、`host.rs` 的 `reconcile`），想改成只看结算帧。

## 实现记录

- 中枢：`SessionState.last_complete` 在 `route_event` 收到 `turn_complete` 时记下（开新一轮时清空）；`settle_turn` 取出它，`harnix.turn_settled`
  多一个可选的 `outcome { stop_reason, usage, structured_output?, message? }`（`crates/hub/src/turns.rs` 的 `outcome_of`）。`result` 不变。
  维护者裁定项「回答文本带不带」：不带，SDK 写明读 `session.history`。
- 模块侧：`Tap::next_settled() -> Option<Value>`（返回 `turn_settled` 的 params，跳过别的帧）。为让落后的 tap 也拿得到，`TapSink` 在通道满时
  把结算帧另存（`SETTLED_HELD = 64` 个），`next_settled` 先读已排队的、再读另存的；`next()` 的行为不变。
- 测试：Rust `turns::outcome_tests::the_outcome_is_the_points_of_turn_complete`（含 `structured_output`、`message`、`stopped_by_hook`）；
  `tests/live/api/turn-outcome.spec.ts` 的「a settled turn says how it ended, as its turn_complete did」（`stop_reason == "end_turn"`，`usage` 与
  `turn_complete` 帧一致）、「a tap that fell behind still has the settlement, with its outcome」（testapp `app.fleet.settle_behind`：容量 1、
  不读的 tap 确实 `Lagged`，`next_settled` 仍拿到与正常 tap 相同的 `outcome`）。
- 偏离：带结构化输出、被 hook 截停的两条 live 用例没写：现有录像里没有这两种轮次（要真模型录新录像），由 Rust 单测覆盖字段的提取；
  用例放在新文件 `turn-outcome.spec.ts`（`turns.spec.ts` 要真模型）。

外壳（F3b）：`ui/packages/core/src/protocol/frames.ts` 的 `Notifications["harnix.turn_settled"]` 补可选的
`outcome?: { stop_reason?, usage?, structured_output?, message? }`（只是类型，外壳不读它）。没有单独的测试：类型由 `typecheck` 覆盖。
