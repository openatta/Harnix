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
- `host.session.open(ref)` 加一个可选参数，让调用方要求「会话在哪个 space 就去哪个 space 打开」：
  ```ts
  open(ref: SessionRef, options?: { switchSpace?: boolean }): void | Promise<void>;
  ```
  `switchSpace: true` 时先 `spaces.switch(会话的 space)` 再打开，返回的 promise 在打开之后落定；会话属于哪个 space 由中枢的账本定
  （`SPACE_MISMATCH` 就是对着它核的），外壳向中枢问（没有现成读法的话，给 `harnix.sessions` 的行带上 `space`，是本条要补的一处）；外壳自己的会话列表只列 `sessions` 这个后端 space（`app.ts:328`），不能拿来判断。
  找不到这个会话就 reject。不带第二个参数时与现在完全一样（包括把别的 space 的会话
  记成前台 space 的当前会话这一点；要不要把它改成报错，另作缺陷处理，不在本条）。

## 兼容性

- `spaces` 是新增成员；`session.open(ref)` 不带第二个参数时，前台 space 的会话照旧打开。
- 不带 `options` 的 `session.open` 行为不变；返回类型放宽成 `void | Promise<void>`，原来不看返回值的调用方不受影响。
- 贡献点 API 版本：`NestHost` 只加成员，可以不升；若维护者按「API 2 加了什么」记账，记一笔即可。
- Helm、hello-app：不改代码。

## 验收

- `tests/live/ui/interface.spec.ts`（testapp 带两个 space A、B）：A 里的组件 `await host.spaces.switch("B")` 之后，`#main[data-space]` 是 B，
  `host.spaces.current()` 是 `"B"`；紧接着 `host.composer.insert("x")`，B 的输入框里有 `x`。
- `switch("nope")` reject。
- 在 A 里 `session.open(B 的会话, {switchSpace: true})`：前台变成 B，打开的是那个会话，A 的当前会话不变。
- 在 A 里 `session.open(A 的会话)` 不带第二个参数：与现在一样（回归）。

## 依赖

无。

## 来源

IPDrun 的对话里的编号、首页流程图的节点都要跳到别的 space；现在由 kit 点左栏按钮、再等 400 ms 或下一帧（`ui/kit/src/shell-bridge.ts`、
`spaces/issues/ui/src/IssuePage.tsx`）。
