# ipdrun-18：space 不能把别的 space 带到前台

类型：新能力｜优先级：高

## 需求

一个 space 里的东西常常指向另一个 space：首页的卡片点进某个 space、对话里的链接跳到它所属的那个 space、通知点开后去对应的 space。
现在宿主 API 里没有切 space 的方法：

- `NestHost`（`ui/packages/react/src/host.tsx:25-63`）没有；外壳内部的 `app.switchSpace`（`ui/packages/shell/src/app.ts:374-385`）不是契约，
  `window.harnix.app` 是给控制台和测试用的；
- `host.session.open(ref)` 只在**前台** space 打开（`shell/src/host.ts:31` → `app.open(key, space = 前台)`，`app.ts:364-368`）。
  会话属于别的 space 时，它被静默记成前台 space 的当前会话，之后的调用报 `SPACE_MISMATCH`（-31003）。

space 只能去点左栏的 DOM 按钮（`Sidebar.tsx:120` 的 `[data-space]`），再用定时器猜切换什么时候完成。

## 建议

```ts
export interface NestHost {
  // …原有成员不变
  spaces: {
    /** 把这个 space 带到前台，带着它自己的当前会话；切换完成（新 space 已画出）后落定。没有这个 space 时 reject。 */
    switch(id: string): Promise<void>;
    /** 前台 space 的 id。 */
    current(): string;
  };
}
```

- `switch` 直接用 `app.switchSpace`（它本来就是 async），在新 space 的第一次渲染之后再 resolve，这样调用方紧接着的 `session.open`、
  `composer.insert` 一定落在新 space 里。
- `host.session.open(ref)` 对别的 space 的会话：**明确报错**（`SPACE_MISMATCH`，消息里写会话属于哪个 space），而不是静默记错位置。
  另给一个可选参数让调用方要求自动切过去：
  ```ts
  open(ref: SessionRef, options?: { switchSpace?: boolean }): void | Promise<void>;
  ```
  `switchSpace: true` 时先 `spaces.switch(会话的 space)` 再打开。会话属于哪个 space，外壳从会话列表（`harnix.sessions` 的 `space` 字段）得知。

## 兼容性

- `spaces` 是新增成员；`session.open(ref)` 不带第二个参数时，前台 space 的会话照旧打开。
- 行为变化只在「打开别的 space 的会话」：以前静默错开、随后 `SPACE_MISMATCH`，现在立刻报同一个错误。这是把延后的错误提前，不是新错误。
- 贡献点 API 版本：`NestHost` 只加成员，可以不升；若维护者按「API 2 加了什么」记账，记一笔即可。
- Helm、hello-app：不改代码。

## 验收

- `tests/live/ui/interface.spec.ts`（testapp 带两个 space A、B）：A 里的组件 `await host.spaces.switch("B")` 之后，`#main[data-space]` 是 B，
  `host.spaces.current()` 是 `"B"`；紧接着 `host.composer.insert("x")`，B 的输入框里有 `x`。
- `switch("nope")` reject。
- 在 A 里 `session.open(B 的会话)`：抛 `SPACE_MISMATCH`，前台仍是 A、A 的当前会话不变；带 `{switchSpace: true}`：前台变成 B，打开的是那个会话。

## 依赖

无。

## 来源

IPDrun 的对话里的编号、首页流程图的节点都要跳到别的 space；现在由 kit 点左栏按钮、再等 400 ms 或下一帧（`ui/kit/src/shell-bridge.ts`、
`spaces/issues/ui/src/IssuePage.tsx`）。
