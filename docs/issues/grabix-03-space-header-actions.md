# grabix-03：space 不能在标题栏放自己的操作（`headerActions`）

类型：新能力｜优先级：中

## 需求

中区标题栏（抬头那一行，`ui/packages/shell/src/Shell.tsx` 的 `Header`）右端只有外壳自己的两个按钮：主题切换（`theme-toggle`）与右栏开合（`details-toggle`）。space 想放与当前会话有关的小控件（用量读数、几个图标按钮），只能塞进消息流或右栏，离会话本身很远。

## 建议

```ts
export interface SpaceContribution {
  // …原有成员不变
  /** 标题栏右端、主题与右栏两个按钮的左边画的内容；没有就返回 null / undefined。
   *  session 是前台会话，没有则为 null。每次画标题栏时读。 */
  headerActions?(session: SessionRef | null): ReactNode;
}
```

- 外壳把结果放进 `<div class="head-actions" data-testid="head-actions">`（横排、间距 4px），包在 `Contributed` 里（失败隔离）：函数本身抛错就没有这一块，返回的组件画时抛错只丢这一块，标题栏其余照常。
- 内容变化由 space 自己的 React 状态驱动；「有没有」的变化用 `host.redraw()`。
- 会话用量等用现有 `useNest().session.context()`（`SessionState`，含 `usage`）。
- 复用外壳的样式，不新增导出组件：按钮用类名 `ib`（32px 方形图标按钮，外壳全局样式），里面放 `<span class="glyph">`；图标用 `registry.icons(...)`（ipdrun-30）注册自己的，名字取 `<owner>-<name>`；文字提示用原生 `title` 属性（外壳本身没有 Tooltip 组件，外壳的按钮也都是 `title`）。

## 兼容性

- 不给 `headerActions` 的 space：标题栏不变。
- 贡献点 API 版本：可选字段，可以不升。
- Helm、hello-app 不受影响。

## 验收

- `tests/live/ui/host.spec.ts`：space 返回一个按钮时，`head-actions` 出现、按钮在 `theme-toggle` 左侧、`theme-toggle` 在 `details-toggle` 左侧，点击能调到 space 自己的函数，参数是前台会话；别的 space 没有 `head-actions`；`headerActions` 抛错的 space 没有这一块，两个外壳按钮仍在。

## 依赖

无（用量读数用现有 `session.context()`）。

## 来源

Grabix 要在标题栏放「上下文用量（已用 / 上限）」和压缩、清理两个图标按钮。

## 实现记录

- `SpaceContribution.headerActions?(session)`（`ui/packages/react/src/contrib.ts`）。
- `Shell.tsx` 的 `Header`：用 `ask(...)` 读（函数抛错回落为 null），结果包在 `Contributed` 与 `head-actions` 容器里，画在 `theme-toggle` 之前。
- 测试：`tests/live/ui/host.spec.ts` 的「headerActions: …」。
