# ipdrun-44：模块侧发消息不能带附件与 `@session` 引用

类型：新能力｜优先级：低

## 需求

界面的输入框发 `harnix.send` 时可以带 `attachments`（上传的文件、粘贴的图片）与 `references`（`@session`，中枢把那个会话的转录作为文本附件，
`crates/hub/src/turns.rs` 的 `send`、`crates/hub/src/spaces.rs` 的 `reference`）。模块侧的 `Sessions::send(&s, text, OnBusy)`
（`crates/module/src/sessions.rs`）只有文本：服务端代人发一条带文件的消息，只能把文件内容拼进正文。

## 建议（只加不改）

```rust
pub enum Attachment { File { path }, Text { path, content }, Image { media_type, data } }   // file() / text() / image()
pub struct SendOptions { pub on_busy: OnBusy, pub attachments: Vec<Attachment>, pub references: Vec<SessionRef> }
ctx.sessions().send_with(&s, "看看这个", SendOptions { attachments: vec![Attachment::file("docs/a.md")], ..Default::default() }).await?;
```

`Attachment` 序列化成引擎认的 `{kind: "file" | "text" | "image", …}`（Agix `runtime::agent::Attachment`）。带附件的不 steer（中枢已有的规则），排到下一轮。

## 兼容性

新增方法与类型；`send` 不变。`SendOptions::default()` 发出的参数与 `send(…, OnBusy::Queue)` 逐字相同。

## 验收

`crates/module` 单测：三种附件与一个引用按引擎的形状进 `harnix.send` 的参数；缺省选项与 `send` 一样，不多带键。

## 依赖

无。

## 来源

IPDrun 的插话框（服务层代人发到会话）要带附件；测试线的入站检查照旧由 Agix 的 prompt 中间件看附件拦（Agix ipdrun-05）。
