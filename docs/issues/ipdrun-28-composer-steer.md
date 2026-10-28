# ipdrun-28：输入框没有「立即送达」（steer）的选项

类型：新能力｜优先级：中

## 需求

中枢早就支持 `harnix.send { on_busy: "steer" }`：一轮在跑时，消息在下一个步骤边界送进这一轮，不必等它结束（`crates/hub/src/turns.rs:146-150`、`:172-193`；
术语见 glossary「steer」）。外壳的输入框却写死排队：`SessionController.send` 固定 `on_busy: "queue"`（`ui/packages/core/src/state/controller.ts:312-326`），
输入框上也没有地方让人选。

长时间跑工具的会话里，人常想在模型干活时插一句「换个方向」「先别动那个文件」，排到这一轮结束再送就晚了。

## 建议

1. `SessionController.send` 加一个可选的第三个参数（只加不改）：
   ```ts
   async send(text: string, attachments: unknown[] = [], options?: { onBusy?: "queue" | "steer" }): Promise<void>;
   ```
   缺省仍是 `"queue"`。`ChatPanel` 相应地加 `onBusy?: "queue" | "steer"`，经它的发送路径传下去。
2. space 声明它的会话要不要这个开关：
   ```ts
   export interface SpaceContribution {
     // …原有成员不变
     /** 输入框的可选项。steer: true ⇒ 输入框底栏有一个「立即送达」开关（只在有一轮在跑时可用）。 */
     composer?: { steer?: boolean };
   }
   ```
   外壳用 `ChatPanel` 现有的 `composerTools` 位（`ui/packages/react/src/chat/ChatPanel.tsx:128`）画这个开关；开着时发送用 `onBusy: "steer"`，
   开关状态按会话记在页面内存里。带附件的消息照中枢现在的规则排队（附件不 steer）。
3. 「排队中」的气泡对 steer 的消息显示「下一步送达」而不是「本轮结束后送达」（`ChatPanel` 的 `queue.note` 已经可以配）。

不提「space 接管发送」：发送前的检查应当在服务端做（`ipdrun-13` 的中间件），界面里的接管挡不住经别的客户端发来的消息。

## 兼容性

- 不声明 `composer.steer` 的 space：输入框与现在一样，没有开关，发送照旧排队。
- `send` 的第三个参数、`ChatPanel.onBusy` 都是可选的新增。
- 协议不变（`on_busy: "steer"` 本来就有）。
- 若应用用 Agix 的 UserPromptSubmit 做检查：在 Agix `ipdrun-04` 落地之前，steer 的消息会绕过它；SDK 里写明。
- 贡献点 API 版本：可选字段，可以不升。Helm、hello-app 不受影响。

## 验收

- `ui/packages/core/test/`：`send("x", [], {onBusy: "steer"})` 发出的 `harnix.send` 带 `on_busy: "steer"`；不带第三个参数时仍是 `"queue"`（回归）。
- `tests/live/ui/conversation.spec.ts`：testapp 的 space 声明 `composer: { steer: true }`；让一轮跑一个慢工具（录像），打开开关发一条消息，
  转录里这条出现在这一轮之内（`user_message_delivered`），而不是新的一轮。
- 不声明的 space：输入框底栏没有这个开关。

## 依赖

无；与 Agix `ipdrun-04`（steer 的消息也过 UserPromptSubmit）一起落地更好。

## 来源

IPDrun 的一类会话里，人要在模型干活时插话并立即送达；现在在右栏另放了一个插话框，走 IPDrun 自己的方法。
