# ipdrun-27：`@` 的候选不能由应用贡献

类型：新能力｜优先级：低

## 需求

输入框里打 `@` 时，外壳给的候选是写死的两类：项目里的文件、`@session:` 后面的会话（打开的会话里，`ui/packages/shell/src/Shell.tsx:555-576`；
新对话首屏只有会话，`:246`）。应用有自己的、值得在对话里引用的东西（文档、数据集、工单、联系人），没有办法把它们加进 `@` 菜单。

`/` 那一侧已经是贡献点（`command`，`ui/packages/react/src/contrib.ts:117`，「全部，按注册顺序拼起来」），`@` 这一侧不是。

## 建议

加一个与 `command` 同形的贡献点 `mention`：

```ts
export interface Contributions {
  // …原有八个不变
  mention: {
    id: string;
    /** 打 `@` 之后的文字 → 候选。ctx 给出前台 space 与打开的会话，贡献可以只在自己的 space 里给。 */
    complete(query: string, ctx: { space: string; session: SessionRef | null }): MentionItem[] | Promise<MentionItem[]>;
  };
}
```

- `MentionItem` 沿用 `ui/packages/react/src/chat/types.ts:62-68`（`id`、`label`、`insert`）；可选加 `source?: string` 用于在菜单里分组。
- 内建的文件、`@session:` 候选改成注册在这个点上的内建贡献（`builtins.tsx`，与 `builtin.commands` 注册在 `command` 上同样的做法），外壳只问这个点：内建的在前，再按注册顺序拼起来，总数封顶；某个贡献抛错或超时只丢它自己的候选。
- 选中后插入 `insert`；它在发送时怎么解释是应用的事（例如由模块经 `ipdrun-13` 的中间件或自己的工具展开）。

`catalog.rs` 的注释说贡献点的个数是设计的一部分（「a seventh is an argument to be made」，`crates/module/src/catalog.rs:59-65`）。理由：
`@` 引用是对话里与 `/` 对称的另一个入口，内建 space 自己就在用（文件、会话），只是没有对应用开放；它也被外壳直接问（`@` 菜单），
不是没人用的接缝。若维护者不想加第九个点，替代方案是给 `command` 的候选加 `trigger?: "/" | "@"`（缺省 `/`），由同一个点承载。

## 兼容性

- 新增贡献点，旧的八个不变；不注册 `mention` 时 `@` 菜单与现在一样。
- `contribution_points.md` 与 `catalog.rs` 的表随之加一行；贡献点 API 版本按维护者的规矩决定是否升（只加不改，可以不升）。
- Helm、hello-app 不受影响。

## 验收

- `ui/packages/react/test/contrib.test.ts`：`mention` 的形状校验（缺 `complete` 被拒并记理由）。
- `tests/live/ui/conversation.spec.ts`：testapp 注册一个 `mention`，对 `ab` 返回 `{id: "doc:ab", label: "文档 ab"}`；输入框里打 `@ab`，菜单里有这一项，选中后插入 `doc:ab `；
  内建的文件候选仍在。
- 贡献抛错：菜单里只少它的候选。

## 依赖

无。

## 来源

IPDrun 想让人在对话里打 `@` 选它自己的需求编号；现在只能手打。
