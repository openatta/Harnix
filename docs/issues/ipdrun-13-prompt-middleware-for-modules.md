# ipdrun-13：模块拿到 Agix 的 prompt 中间件

类型：新能力｜优先级：中

## 需求

模块想在「人的消息进一轮之前」按自己的规则放行、改写或拒收，并且对**经外壳直接发给会话的消息**也生效（`harnix.send` 不经过模块）。
今天模块能用的原生接缝是 `EngineContrib::customize_session`（`crates/instance/src/contrib.rs:33-40`），它把 Agix 的
`runtime::agent::Builder` 交给模块，工具调用可以挂 `Builder::tool_middleware`；用户消息这一侧 Agix 没有对应物，模块只能退回
设置层的 command hook（每条消息起一个子进程）。

Agix `ipdrun-03` 建议给 `Builder` 加 `prompt_middleware`。它落地后，Harnix 这边要做的是让模块用得上、两种载体一致。

## 建议

1. 升 `agix/` 的钉到带 `prompt_middleware` 的 Agix。模块在 `customize_session` 里直接调：
   ```rust
   fn engine(&self, e: &mut EngineContrib) {
       e.customize_session(|b, _cx| b.prompt_middleware(Arc::new(MyPolicy)));
   }
   ```
   不需要新的 Harnix 接口。
2. `harnix_module` 再导出 `PromptMiddleware`、`PromptCall`、`PromptDecision`、`PromptDelivery`（与 `EngineContrib` 一样从
   `harnix_module` 取），模块不必直接依赖 Agix 的 crate 路径。
3. spawn 实例要同样生效，依赖 `ipdrun-10`（spawn 模板带原生贡献）：同一个 `native` 闭包在 daemon 里再跑一遍。
4. 被拒的消息，中枢要把结果交给客户端：
   - 开一轮时被拒：照现在的 `turn_complete`（`stop_reason = "stopped_by_hook"`、`message`）原样转发，外壳在流里显示 `message`；
   - steer 时被拒：Agix `ipdrun-04` 的 `user_message_refused` 事件原样转发，外壳把那条从「排队中」里拿掉并显示理由。
5. SDK §6.1 加一小节「拦截人的消息」，写明次序（设置层 hook 之后、按模块注册顺序）与两种载体的差别（spawn 时中间件跑在 daemon 进程里）。

## 兼容性

- 不用这个中间件的模块：没有变化。
- 升 Agix 钉本身的影响按 Agix 那一版的说明；`EngineContrib`、`Module` trait 不变，`MODULE_API_VERSION` 不必升。
- 外壳多认一个事件 `user_message_refused`；没有模块用中间件时它不会出现。
- Helm、hello-app 不受影响。

## 验收

- `crates/testapp` 的 `engine()` 挂一个中间件：消息含 `blocked-word` 时 Block，其余放行。
- `tests/live/api/turns.spec.ts`：设备 `harnix.send` 一条含 `blocked-word` 的消息 → `harnix.turn_settled` 到达，结果里 `stop_reason == "stopped_by_hook"`，
  录像里 0 次模型调用；下一条普通消息正常跑完。
- 一轮在跑时 `on_busy: "steer"` 发 `blocked-word`：客户端收到 `user_message_refused`，转录里没有这条。
- spawn 载体同样一遍（`ipdrun-10` 落地后）。
- `tests/live/ui/conversation.spec.ts`：被拒的那条在流里显示理由。

## 依赖

Agix `ipdrun-03`（必需），Agix `ipdrun-04`（steer 的拒收），Harnix `ipdrun-10`（spawn 载体）。

## 来源

IPDrun 要对人直接说给某类会话的话先做检查；现在两种载体都走 command hook（`ipdrun user-prompt` 子进程），宿主自己发的消息还得先登记、
让 hook 放行（`crates/agent/src/hook.rs`）。
