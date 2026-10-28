# ipdrun-01：单双向流上超过在途上限的请求被静默丢掉

类型：缺陷｜优先级：高

## 现象

单双向流（`/ws`）每条连接一个 `Semaphore::new(MAX_IN_FLIGHT)`（16，`crates/transport/src/ws.rs:33`、`:170`）。
拿不到许可时：

```rust
let Ok(permit) = permits.clone().try_acquire_owned() else {
    warn!("client exceeded its in-flight budget; frame dropped");
    continue;
};
```
（`ws.rs:314-317`）

请求帧被丢掉，**不回错误帧**。客户端 `HarnixClient.call()` 的 promise 永远不落定（`ui/packages/core/src/client/client.ts:398-409`
只在应答或断线时清 `pending`）。一个 space 的界面在几毫秒里收到几条事件、各自重取数据时，很容易瞬时超过 16 个在途调用；
被丢的那一次没有错误、没有超时，界面停在旧值或一直转圈。外壳自己的调用也走这条连接，space 无从知道预算还剩多少。

`hello` 公布的 `Limits`（`crates/contract/src/handshake.rs:72-77`）里也没有这个上限，客户端没法事先遵守。

## 建议

1. **服务端：超额回错误帧，不丢。** 在 Harnix 自己的码段加一个码：
   ```rust
   // crates/contract/src/lib.rs, mod codes
   /// This connection already has as many requests in flight as it may.
   pub const BUSY: i32 = -31005;
   ```
   超额时回 `{"code": -31005, "message": "16 requests in flight on this connection; retry when one answers", "data": {"max_in_flight": 16}}`。
   通知帧（无 `id`）超额仍可丢弃，但要计数进诊断。
2. **公布上限。** `Limits` 加 `max_in_flight: usize`（`run.rs:226-230` 组装处一起填），客户端在 `hello` 里读到。
3. **客户端排队。** `HarnixClient` 按 `limits.max_in_flight`（没有就按 16）在本地排队：在途满了，新调用等前一个落定再发。
   这样外壳和所有 space 共用一份预算，谁也不必自己数。可以给 `HarnixClientOptions` 加 `maxInFlight?: number` 覆盖。
4. 分离流（`/rpc`）若有同样的上限，行为对齐；没有就在 `Limits` 的文档里写明它只约束单双向流。

## 兼容性

- 线上行为：以前被丢的请求现在收到错误帧。以前它们永远悬着，没有调用方能依赖那种行为。
- 新错误码在 Harnix 的码段（`-31000` 起），不与 Agix 的码冲突；`ts_export` 的码表随之导出。
- `Limits` 多一个字段：TS 侧是新增可选读取；Rust 侧只有 Harnix 自己构造它。协议版本不必升。
- 客户端排队后，同一时刻发出的请求不会超过上限，正常情况下服务端的 `BUSY` 不会再出现；直接写 WebSocket 的第三方客户端会看到它。
- Helm、hello-app：不改代码；高并发时调用从「偶尔悬住」变成「排队完成」。

## 验收

- `tests/live/api/limits.spec.ts`：同一连接上一次发 40 个慢调用，每个都在超时内落定——要么成功，要么 `-31005` 且 `data.max_in_flight == 16`；
  没有一个悬着。
- `ui/packages/core/test/client.test.ts`：模拟服务端只许 2 个在途，连发 10 个，`client` 最多同时送出 2 个，10 个全部落定。
- `hello` 的 `limits.max_in_flight` 与服务端常量一致（`limits.spec.ts`）。

## 依赖

无。

## 来源

IPDrun 的一个界面在接受一次变更时收到 6 条事件，各个列表同时重取，第 17 个请求被丢，列表一直停在旧状态；
IPDrun 现在在自己的数据层里限 8 个在途（`ui/kit/src/data/client.ts`）。

## 实现记录

后端（F3a）：

- `codes::BUSY = -31005`（`crates/contract/src/lib.rs`）；`Limits.max_in_flight`（常量 `MAX_IN_FLIGHT = 16`，`run.rs` 组装处填），`hello` 里公布；
  生成的 `ui/packages/core/src/protocol/generated/contract.ts` 随之重新生成（多了 `BUSY` 与 `max_in_flight`）。
- `ws.rs`：超额的请求回 `{"code": -31005, "data": {"max_in_flight": 16}}`；超额的通知照旧丢弃，计数进 `harnix.diagnostics` 的
  `transport.dropped_notifications`。
- 第 4 条：分离流（`/rpc`）没有在途上限，写进 `Limits.max_in_flight` 的文档，不加上限。
- testapp 的 `probe` 加了慢调用 `app.probe.sleep {ms}`。
- 测试：`tests/live/api/limits.spec.ts` 的「a request over the in-flight cap is answered BUSY, on a raw connection」（裸 WebSocket 同时发 40 个
  400 ms 的调用，全部在时限内落定，超额的是 `-31005` 且 `data.max_in_flight` 与 `hello` 一致）；「hello reports every ceiling」改为 4 个上限；
  原「more requests in flight than the cap are answered」改为接受 `BUSY`。

外壳部分待做（建议第 3 条与客户端测试）：`HarnixClient` 按 `limits.max_in_flight`（缺省 16）本地排队、`HarnixClientOptions.maxInFlight?`；
`ui/packages/core/src/protocol/frames.ts` 手写的 `Limits` 补 `max_in_flight?`；`ui/packages/core/test/client.test.ts` 的排队用例。
