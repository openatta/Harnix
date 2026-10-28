# ipdrun-27：`@` 的候选不能由应用贡献

类型：新能力｜优先级：低

## 需求

输入框里打 `@` 时，外壳给的候选是写死的两类：项目里的文件、`@session:` 后面的会话（打开的会话里，`ui/packages/shell/src/Shell.tsx:555-576`；
新对话首屏只有会话，`:246`）。应用有自己的、值得在对话里引用的东西（文档、数据集、工单、联系人），没有办法把它们加进 `@` 菜单。

`/` 那一侧已经是贡献点（`command`，`ui/packages/react/src/contrib.ts:117`，「全部，按注册顺序拼起来」），`@` 这一侧不是。

## 建议

**不加新的贡献点。** `crates/module/src/catalog.rs:59-65` 说贡献点的个数是设计的一部分，新增一个要先论证；`@` 候选和 `/` 候选是同一件事
（打一个前导字符 → 按输入的文字给一串「插入什么、显示什么」的候选，全部按注册顺序拼起来），形状与 `command` 完全相同，所以由 `command`
承载，只加一个可选字段：

```ts
export interface Contributions {
  command: {
    id: string;
    /** 哪个前导字符的菜单问它；缺省 "/"（现在的行为）。 */
    trigger?: "/" | "@";
    complete(query: string, ctx?: { space: string; session: SessionRef | null }): CommandCandidate[] | Promise<CommandCandidate[]>;
  };
}
```

- `trigger` 放在贡献上、不放在候选上：`/` 菜单现在是注册表变化时问一次 `complete("")`、再在本地过滤（`Shell.tsx:652-683`），`@` 菜单是
  每次输入都问（`mention(query)`，要查文件）。按贡献分开，外壳问 `/` 时不碰 `@` 的贡献，反之亦然。
- `@` 菜单：外壳内建的文件、`@session:` 候选在前，再按注册顺序拼上 `trigger: "@"` 的贡献的候选，总数封顶；某个贡献抛错或超时只丢它自己的候选
  （与 `command` 现在的失败规则相同）。候选的 `replacement` 就是插入的文字（对应 `MentionItem.insert`），`label`、`description` 照画。
- 内建的文件、会话候选要不要也改成注册在 `command`（`trigger: "@"`）上的内建贡献，由维护者定；不改也满足「点被内建注册、被外壳问」。
- `ctx` 与 `ipdrun-29` 的是同一个，贡献可以只在自己的 space 里给候选。
- 选中后插入 `replacement`；它在发送时怎么解释是应用的事（例如由模块经 `ipdrun-13` 的中间件或自己的工具展开）。
- `catalog.rs` 里 `command` 一行的「what」改成同时说 `/` 与 `@`，`contribution_points.md` 随之重新生成；点的个数不变。

## 兼容性

- 不写 `trigger` 的 `command` 贡献：`trigger` 视为 `"/"`，与现在一样；不注册 `trigger: "@"` 的贡献时 `@` 菜单与现在一样。
- `POINTS` 与 `SHAPE` 不变（`trigger` 可选，形状校验只查 `complete`）。
- 贡献点 API 版本：只加可选成员，可以不升。Helm、hello-app 不受影响。

## 验收

- `ui/packages/react/test/contrib.test.ts`：带 `trigger: "@"` 的 `command` 贡献照常注册；`trigger` 取别的值被拒并记理由。
- `tests/live/ui/conversation.spec.ts`：testapp 注册一个 `trigger: "@"` 的 `command`，对 `ab` 返回 `{replacement: "doc:ab ", label: "文档 ab"}`；
  输入框里打 `@ab`，菜单里有这一项，选中后插入 `doc:ab `；内建的文件候选仍在；`/` 菜单里没有它。
- 不带 `trigger` 的 `command` 贡献：只出现在 `/` 菜单（回归）。
- 贡献抛错：菜单里只少它的候选。

## 依赖

`ctx` 与 `ipdrun-29` 一起做最省事；不依赖它也能先做（不传 `ctx`）。

## 来源

IPDrun 想让人在对话里打 `@` 选它自己的需求编号；现在只能手打。

## 实现记录

外壳（F3b）：

- `command` 贡献加 `trigger?: "/" | "@"`，别的值注册时被拒、理由进 `refusals()`；`POINTS`、`SHAPE` 不变。`/` 菜单只问不带 `trigger` 或 `"/"` 的，
  `@` 菜单（新对话首屏与打开的会话两处）先给外壳自己的（文件、`@session:`），再按注册顺序拼 `trigger: "@"` 的贡献的候选（`complete(query, ctx)`），
  一个贡献抛错或 1.5 s 不答只丢它自己的、记一条失败；外壳那边总数封顶 20，输入框显示前 8 个。候选映射为 `MentionItem {id: label, label: description, insert: replacement}`。
- 内建的文件、会话候选没有改成注册在 `command` 上的内建贡献（issue 说由维护者定，不改也满足）。
- `crates/module/src/catalog.rs` 里 `command` 一行的「what」改为同时说 `/` 与 `@`，`docs/contribution_points.md`、概念文档 §5 的表随之改（点的个数不变；
  对齐它的 Rust 测试由全量检查跑）。
- 测试：`ui/packages/react/test/contrib.test.ts` 的「takes a command for the @ menu, and refuses a trigger that is neither / nor @」；`tests/live/ui/host.spec.ts` 的
  「@ takes an application's candidates after the shell's own, and the / menu does not」（含一个抛错的贡献只丢自己的）。
