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

## 实现记录

外壳（F3b）：

- `SpaceContribution.badge?()`；左栏入口（顶部与 `place: "bottom"` 的都走新的 `SpaceEntry`）在标题后画 `.end > .bn[data-testid=space-badge]`，`aria-label` 是
  「space 名：值」；收起成图标条时沿用已有的 `.bn` 规则，缩成右上角的小圆点。null / undefined / 0 / "" 不画。
- 函数抛错：只是不画，space 记一条失败（`app.ts` 的 `ask`：在这次绘制之后再记，免得画的时候改注册表又触发一次绘制）。顺带：注册表的 `refuse` 只在记下新的一条时
  才通知，同一个贡献每次绘制都失败不再引起重画。
- 测试：`tests/live/ui/host.spec.ts` 的「badge: a space's entry shows its count, not when it is zero, also on the rail; one that throws is noted」；
  `ui/packages/shell/test/host.test.ts` 的「give the fallback when they throw, and the space is noted after the draw」。
