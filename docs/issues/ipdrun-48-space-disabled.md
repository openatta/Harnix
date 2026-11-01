# ipdrun-48：space 不能在左栏暂时灰掉（`SpaceContribution.disabled`）

类型：新能力｜优先级：中

## 现象或需求

左栏的 space 入口（`ui/packages/shell/src/Sidebar.tsx` 的 `SpaceEntry`）总能点。一个 space 若在某些时候没有可看的东西
（它依赖的数据还没就绪、前提条件不满足），只能整个注销再注册（列表跳动，`order` 之外的状态丢掉），
或者留着让人点进去看一页「现在不能用」。`host.spaces.switch` 也没法拒绝；页面重新载入时（`App.landOnASpace`，
`ui/packages/shell/src/app.ts:444`）存着的 space 若此刻不该进，也会落在它上面。

## 建议

```ts
interface SpaceContribution {
  /** 给一句话 ⇒ 这个 space 暂时不能进：左栏灰掉、点不动，悬停提示是这句话。false / null / undefined ⇒ 能进。 */
  disabled?(): string | false | null | undefined;
}
```

- 每次画都读（同 `badge()`，抛错只算这个 space 失败、当作能进）；状态变了调 `host.redraw()`。
- 返回字符串：入口加 `disabled` 类、`aria-disabled="true"`、`data-disabled`，`title` 是这句话，点了不切（不用 `disabled` 属性：有的浏览器在禁用的按钮上不显示悬停提示）。外壳自己找「能新建会话的 space」时也跳过灰着的。
- `host.spaces.switch(id)` 到灰着的 space：不切、不报错，promise 照常落定（调用方用 `spaces.current()` 看结果）。
  拦在左栏点击与 `host.spaces.switch` 这两个入口；`App.switchSpace` 本身不拦（外壳内部的其他切换照旧）。
- 落地（连上后第一次选 space）：存着的 space 灰着或已不在 ⇒ 落到 `order` 最小、没灰的那个；全都灰着 ⇒ 存着的还在就留在它上面，不在就 `order` 最小的那个。
- 前台 space 后来灰掉（`redraw` 时）：外壳不自动换走，由应用决定（例如自己 `spaces.switch` 到别处）。

## 兼容性

可选字段；不写的 space 照旧，`spaces.switch` 到它照旧。`host.spaces.switch` 只在写了 `disabled()` 且此刻返回字符串时才不切。
贡献点 API 版本不必升。

## 验收

`ui/packages/shell/test/nav-group-disabled.test.tsx`：

- `disabled()` 返回一句话：入口带 `disabled` 类、`aria-disabled`、`title` 是这句话，点它不切；改成 `false` 再 `redraw()` ⇒ 恢复，点了就切。
- `host.spaces.switch` 到灰着的 space：落定、前台不变；不写 `disabled()` 的照旧切过去。
- `disabled()` 抛错：入口照常可点，诊断里记一条。
- 落地：存着的 space 灰着 ⇒ 落到 `order` 最小、没灰的；没灰 ⇒ 留在存着的。

## 依赖

即时刷新要 ipdrun-19 的 `host.redraw()`。

## 来源

IPDrun R11 「项目」space：stage-R11.md §1.10。没有打开的项目时，项目内的类目 space 都要灰着，悬停说「先打开一个项目」。

## 落地

（待填）
