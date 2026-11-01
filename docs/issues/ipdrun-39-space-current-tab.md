# ipdrun-39：space 说不了自己当前是哪个页签，换回来总在第一个

类型：新能力｜优先级：中

## 现象或需求

右栏选中的页签按 space 记在 `ShellState.tabBySpace`（ipdrun-19）。换 space 时（`ui/packages/shell/src/app.ts:475`），
人点的页签作废、回到第一个；只有经 `host.details.open(tab)` 选的留着。一个 space 若自己记着「当前在哪一页」
（例如列表 → 打开的一项），换回来时想让右栏落在那一页，只能在 `spaces.onChange` 一类的时机再调一次 `details.open(tab)`——
那会顺带打开右栏，时机也晚一帧。

## 建议

```ts
interface SpaceContribution {
  /** 本 space 当前的右栏页签：它的页签 id，或 "details"。 */
  tab?(): string | undefined;
}
```

- space 被带到前台时、`host.redraw()` 时读它；返回本 space 的一个页签 id（或 `"details"`）就选中它（同 `details.open(tab)` 记的那样，
  但不打开右栏）；`undefined` 或不认识的 id：外壳自己的选择不变。抛错只算这个 space 失败。
- 写了 `tab()` 的 space，人点的页签换 space 回来也还在（外壳不再替它丢掉）。
- 没写 `tab()` 的 space 完全照旧，包括「人点的页签换 space 后回到第一个」。

## 兼容性

可选字段。ipdrun-19 定下「人点的页签换 space 回到第一个」并有测试（`ui/packages/shell/test/host.test.ts`「a clicked tab still goes back
to the first…」）盯着，所以「记住人点的页签」只对写了 `tab()` 的 space 生效，缺省行为不变。贡献点 API 版本不必升。
写了 `tab()` 的 space 要自己跟着 `details.onTab` 更新，否则下一次 `redraw()` 会把人点的页签换回它说的那页。

## 验收

`ui/packages/shell/test/space-tab.test.ts`：

- `tab()` 返回 `t2`，换到这个 space ⇒ 选中 `t2`、右栏不打开；改成 `t3` 再 `redraw()` ⇒ `t3`；不认识的 id ⇒ 不变；`onTab` 各收到一次。
- `tab()` 返回 `undefined`：落在第一个；人点 `t3`，换走再回来仍是 `t3`。
- `tab()` 抛错：落在第一个，诊断里记一条。
- 原有 `host.test.ts` 的右栏用例照旧通过。

## 依赖

建在 ipdrun-19 上。

## 来源

IPDrun 的类目 space 记着自己的页签栈与当前页（UF-04），切回类目时要选回它（`ui/kit/src/compat/tabs.tsx`，HARNIX-WORKAROUND C1-#7）。

## 落地

落地提交：`186186f`（main，2026-11-01，未推送；版本待下次发布）。
外壳：`App.followSpaceTab`（`switchSpace` 与 `redraw` 之后调），`switchSpace` 对写了 `tab()` 的 space 不丢人点的页签（`ui/packages/shell/src/app.ts`）。
与需求原文的差别：「记住人点的页签」没有对所有 space 打开（见兼容性）。
