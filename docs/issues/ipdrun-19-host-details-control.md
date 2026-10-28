# ipdrun-19：space 管不了右栏：选哪个页签、开合、页签变了让外壳重画

类型：新能力｜优先级：高

## 需求

右栏的页签由 space 给（`SpaceContribution.tabs`，可以是函数，`ui/packages/react/src/contrib.ts:94-96`；外壳每次画时读，`tabsOf`，`:69-73`）。
2701 起 space 能定右栏第一次进来时开不开（`details?: "open" | "closed"`），之后按 space 各记各的（`ui/packages/shell/src/app.ts:417-437`）。
还缺三样：

1. **选中哪个页签。** 选中的页签是外壳 `Details` 组件的局部 state（`ui/packages/shell/src/Shell.tsx:704-713`）：换 space 时回到第一个，
   揭示一个详情时跳到「详情」。space 打开了一个新页签（例如用户在列表里点开一项，space 的 `tabs()` 多出一页），想让它成为选中的那页，没有办法。
2. **开合。** space 想在某个动作之后把右栏打开或收起（例如点开一项时打开），只能去点 `[data-testid=details-toggle]`。外壳有
   `app.toggleDetails(open?)`，但不在宿主 API 里。
3. **重画。** `tabs` 是函数时，外壳只在自己重画时读它；space 的状态变了、页签该多一页或少一页时，没有办法让外壳重读。现在只能注册一个空语言包，
   让注册表版本号 +1 去触发 `useRegistry`。

另外，人点了某个页签，space 也不知道（想让自己的状态跟着选中项走时需要）。

## 建议

```ts
export interface NestHost {
  // …原有成员不变
  details: {
    /** 打开右栏；给了 tab 就选中它（这一页必须在当前 space 的 tabs 里，或是 "details"）。 */
    open(tab?: string): void;
    close(): void;
    isOpen(): boolean;
    /** 当前选中的页签 id。 */
    tab(): string;
    /** 选中的页签变了（人点的或 open(tab) 引起的）。返回退订。 */
    onTab(fn: (tab: string) => void): () => void;
  };
  /** 前台 space 的函数（tabs、title、notice…）变了，让外壳重读。 */
  redraw(): void;
}
```

- 选中的页签从 `Details` 的局部 state 挪进 `ShellState`（按 space 记，和开合一样），`open(tab)` 与点击都写它；没有被要求时保持现在的规则
  （换 space 回到第一个，揭示详情跳到「详情」）。
- `open(tab)` 给了一个此刻不在 `tabs()` 里的 id：先 `redraw()` 再找；还找不到就只打开、不换页签，并在诊断里记一条。
- `redraw()` 只让外壳重新渲染读 space 函数的那几处（`Details`、标题、`notice` 等），不改注册表版本号。
- 可选：`SpaceContribution` 加 `tab?(): string | undefined`，让 space 自己说「现在应该选中哪一页」（受控），外壳每次画时读；与 `details.open(tab)` 二选一即可，
  由维护者定。

## 兼容性

- 全是新增成员；不调它们的 space 行为与现在一致，包括「换 space 回到第一个页签」「揭示详情跳到详情页」。
- 开合仍按 space 存在 `harnix.details`，`open` / `close` 写的是同一份。
- `data-tab`、`details-toggle` 这些 DOM 属性不变，外部测试的选择器照旧可用。
- 贡献点 API 版本：只加成员，可以不升。
- Helm：它的 space 用 `tabs` 与 `details`，不改代码；以后想程序化选页签时可以用新接口。

## 验收

`tests/live/ui/interface.spec.ts`（testapp 的 space 的 `tabs()` 依赖一个计数器）：

- 计数器 +1 后调 `host.redraw()`：右栏多出一页，不必任何 DOM 操作。
- `host.details.open("t2")`：右栏打开、`t2` 是 `aria-selected="true"`；`host.details.tab() === "t2"`。
- 人点 `t1`：`onTab` 收到 `"t1"`。
- `host.details.close()` 后 `#details[aria-hidden="true"]`；换到别的 space 再回来，开合与选中页签各自保持。
- 不调任何新接口时，现有的 `interface.spec.ts` 用例全部照旧通过（回归）。

## 依赖

无。

## 来源

IPDrun 的 space 把「列表 → 打开的一项 → 再下一层」画成右栏的一串页签，打开一项时要选中新的那页；现在靠注册空语言包触发重画、下一帧点
`[data-tab]`、收着时点 `details-toggle`（`ui/kit/src/shell-bridge.ts`、`ui/kit/src/keys.ts`）。
