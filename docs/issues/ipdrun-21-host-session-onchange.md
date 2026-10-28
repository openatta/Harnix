# ipdrun-21：前台会话变了，space 收不到通知

类型：新能力｜优先级：中

## 需求

`useNest().session.current()` 只能读（`ui/packages/react/src/host.tsx:30-32`，外壳实现 `ui/packages/shell/src/host.ts:26-29`）。
人在左栏点了另一个会话、新建了一个会话、当前会话被删了，space 都不知道。想让右栏跟着当前会话走（显示这个会话相关的东西），
space 只能自己接管 `sidebar()` 把会话列表全画一遍、自己处理点击，丢掉外壳的会话列表。

## 建议

```ts
export interface NestHost {
  session: {
    // …原有成员不变
    /** 前台 space 的当前会话变了（打开、新建、关掉、被删、切了 space）。返回退订。 */
    onChange(fn: (current: SessionRef | null) => void): () => void;
  };
}

// @harnix/react 里再给一个 hook，内部就是 current() + onChange
export function useCurrentSession(): SessionRef | null;
```

外壳在 `currentBySpace[前台]` 或前台 space 变化时（`ui/packages/shell/src/app.ts:364-385`、`:388-395`）通知；同一个值不重复通知。

## 兼容性

- 纯新增；不订阅的 space 没有变化。
- 贡献点 API 版本：只加成员，可以不升。
- Helm、hello-app 不受影响。

## 验收

- `ui/packages/shell/test/` 加 host 的单元测试：`app.open(a)` → 监听者收到 `a`；再 `open(a)` 不重复；`open(null)` 收到 `null`；`switchSpace` 到另一个
  有当前会话的 space，收到那个会话。
- `tests/live/ui/interface.spec.ts`：testapp 的 space 用 `useCurrentSession()` 在右栏显示当前会话 id；在左栏点另一个会话，右栏跟着变。

## 依赖

无。

## 来源

IPDrun 要「在左栏点一个会话 → 右栏跟到这个会话对应的那一项」；现在只能反过来做（右栏选中 → 打开会话），左栏点会话时右栏不动。
