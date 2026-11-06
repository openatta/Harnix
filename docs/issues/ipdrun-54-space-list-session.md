# ipdrun-54：space 的主会话在左栏会话列表里与 space 入口重复（`SpaceContribution.listSession`）

类型：新能力｜优先级：低

## 现象或需求

IPDrun 的每个类目 space 有一个主会话（类目会话），点左栏的 space 入口就打开它；可它也按时间排在下面的会话列表里，
名字与 space 相同（「需求」入口下面又是一行「需求」），选中时两行都高亮，看起来是重复的标题。应用要能说"这个会话不列"。

## 建议

`SpaceContribution.listSession?(session: SessionRef): boolean`：没有 `sidebar()` 的 space，外壳画它的会话列表时逐个问，
返回 false 的不列；每次画都读（答案变了应用调 `host.redraw()`）；没给、或抛错 ⇒ 照列。全部被拿掉时什么都不画（不出"还没有会话"）。

## 兼容性

只加可选字段；不给的 space 照旧。

## 验收

shell 的 vitest 过（false 不列、抛错照列、全拿掉不出空提示）；IPDrun 各类目 space 的会话区不再出类目会话。

## 落地

落地提交：见 README 总表（main，2026-11-06）。
