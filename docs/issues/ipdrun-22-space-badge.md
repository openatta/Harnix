# ipdrun-22：space 在左栏不能显示角标

类型：新能力｜优先级：中

## 需求

左栏的 space 入口只有图标和标题（`ui/packages/shell/src/Sidebar.tsx:115-128`）。一个 space 想告诉人「这里有 3 件事等你」（待处理数、
未读数、出错的任务数），只能把数字拼进 `title()`（例如「收件箱 · 3」），既不好看，收起左栏变成图标条时也看不到。

## 建议

`SpaceContribution` 加一个可选函数，与 `title()`、`notice()` 一样每次画时读：

```ts
export interface SpaceContribution {
  // …原有成员不变
  /** 左栏入口上的角标：数字或很短的文字；null / undefined / 0 / "" 不画。 */
  badge?(): string | number | null | undefined;
}
```

- 外壳在入口右侧画一个小角标（展开时在标题后，收起成图标条时叠在图标右上角），有 `data-testid="space-badge"`、`aria-label` 带上数值。
- 数值变了由 space 调 `host.redraw()`（`ipdrun-19`）通知外壳重读；在 `ipdrun-19` 之前，外壳可以在每次注册表或会话列表变化时顺带重读。
- 样式用主题已有的令牌（强调色、圆角），应用可以用令牌覆盖改颜色。
- 底部 space（`place: "bottom"`）同样支持。

## 兼容性

- 不给 `badge` 的 space：没有变化。
- `SHAPE` 校验（`contrib.ts:121-130`）只查必有字段，不受影响。
- 贡献点 API 版本：可选字段，可以不升。
- Helm、hello-app 不受影响。

## 验收

- `tests/live/ui/interface.spec.ts`：testapp 的 space 的 `badge()` 返回一个计数器；为 0 时左栏这个入口没有 `space-badge`；+1 并 `redraw()` 后出现、内容是 `1`；
  左栏收起后角标仍可见。
- `badge()` 抛错：只是这个角标不画，入口照常，诊断页记一条（与其他贡献函数失败时的规则一致）。

## 依赖

数值变化后的即时刷新依赖 `ipdrun-19` 的 `host.redraw()`；没有它也能先做（随下一次重画更新）。

## 来源

IPDrun 的首页要显示「等人处理的事」的个数，现在把数字拼进 space 的标题（`spaces/home/ui/src/hooks.ts`）。
