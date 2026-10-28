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

## 实现记录

（F3c，与 ipdrun-07、ipdrun-10 同一提交，提交号见 `git log --grep ipdrun-13`；前置：`bbdd8e2` 把 `agix/` 钉到 `Agix-v0.2.9-2026102701`。）

- 升钉后 Harnix 不用改就能编过：中枢没有对 `AgentEvent` 的穷举匹配，会话事件不论种类都经 `harnix.event` 原样转发，所以 `user_message_refused`
  （`{message_id, content, message}`）直接到客户端；中枢不把它放回队列（它不像 `user_messages_returned`）。开一轮时被拒走原来的
  `turn_complete`（`stopped_by_hook`），`harnix.turn_settled.outcome` 里有 `stop_reason` 与 `message`（ipdrun-17）。
- `harnix_module` 与 `harnix` 再导出 `PromptMiddleware`、`PromptCall`、`PromptDecision`、`PromptDelivery`。模块照建议
  `e.customize_session(|b, _| b.prompt_middleware(Arc::new(…)))`；spawn 载体经 ipdrun-10 的 `native`。
- SDK §6.1 加「拦截人的消息」（次序、两种结局、两种载体），参考文档补再导出与 `EngineContrib` 一行。
- 测试：testapp 的 `probe` 在 `engine()` 里挂 `Blocklist`（含 `blocked-word` 就 `Block`），`fleet` 的 `native` 也挂它。
  `tests/live/api/native-contrib.spec.ts`：「system: a module's prompt middleware refuses a turn, and the next one runs」（`stopped_by_hook`、
  `turn_complete.api_calls == 0`、下一条正常 `end_turn`）、「system: a steered message the middleware refuses arrives as user_message_refused, and stays
  out of the transcript」（录像 `runs-a-while` 让一轮跑 4 s，`on_busy: "steer"` 发含 `blocked-word` 的话：收到 `user_message_refused`，`session.history`
  里没有它）、「spawn: the template's prompt middleware refuses a turn before the model」「in_process: …」（两种载体）。

偏离：

- 用例放在新文件 `native-contrib.spec.ts`（testapp 后端、回放），没放进 `turns.spec.ts`：后者只在 `--live` 下跑、用的是空应用。
- steer 拒收只在 `system`（进程内）测了；spawn 载体测的是开一轮时的拒收。
- 界面用例没写（见下）。

外壳部分待做（已做，见下面「实现记录（外壳部分）」）：

- `ui/packages/core/src/protocol/frames.ts` 的事件联合里加 `{ kind: "user_message_refused"; message_id: string; content: string; message?: string | null }`；
  `ui/packages/core/src/state/model.ts` 里收到它时把对应的「排队中 / 待送达」那条（按 `message_id`，没有就按 `content`）拿掉，并在流里显示
  `message`（被拒的理由）。
- 开一轮被拒（`turn_complete.stop_reason == "stopped_by_hook"` 且带 `message`）时，流里显示那条理由（现在若只当作普通结束，没有提示）。
- 界面用例 `tests/live/ui/conversation.spec.ts`：testapp 后端上发含 `blocked-word` 的话，流里看得到理由；steer 时被拒的那条从排队中消失并显示理由
  （录像 `runs-a-while`，要先允许那次 Bash）。

## 实现记录（外壳部分）

（F3d，提交号见 `git log --grep ipdrun-13` 中标题为 "ipdrun-13: the shell shows a refused message" 的一条。）

- `frames.ts`：事件联合加 `UserMessageRefusedEvent`（`{kind: "user_message_refused", message_id, content, message?}`）。
- `model.ts`：收到它时从「待送达」（`steering`）里拿掉那条——先按 `message_id`，对不上再按 `content`——并在流里加一行
  `note`（`tone: "stop"`）说理由和被拒的那句；它不成人的话，也不碰队列（队列是中枢的整值快照，steer 拒收不会回到队列）。
  `turn_complete` 的 `stop_reason == "stopped_by_hook"` 且带 `message` 时，那一行说理由，不再只说「停止原因：stopped_by_hook」；
  不带 `message`、别的停止原因都照旧。
- 文案：`SessionStrings.messageRefused?`、`HarnixUiStrings.messageRefused?`（可选，旧的整张文案表不用改；没给时只显示理由），
  `ApplyOptions.refusedText?`；zh-CN「「…」被拒收：理由」，en「"…" was refused: reason」。
- 测试：`ui/packages/core/test/state-model.test.ts` 的「人的话被拒收（ipdrun-13）」三条（按 message_id / 按内容拿掉、开一轮被拒说理由、
  不带 message 照旧）；`tests/live/ui/conversation.spec.ts` 的 describe「a message a module refuses」两条：
  「refused at the start of a turn, the flow says why, and the next message runs」「refused when steered into a running turn, it leaves the
  pending list and the flow says why」（testapp 后端；steer 在权限卡挂着时发出，允许之后在步骤边界被拒）。

偏离：

- 中枢今天不发 `delivery: "steer"` 的 `user_message`，外壳也不按 `harnix.send` 的 steer 应答登记「待送达」，所以 steer 的那条平时不在
  `steering` 里；拿掉的逻辑照写（有就拿掉），界面用例验证的是「排队中、待送达都没有它、流里没有它这条人的话、有理由」。是否让外壳按 steer
  应答登记待送达，是另一件事，没在这里改（会改变没有拒收时的显示）。
