# ipdrun-50：窄屏时右栏只能盖住中区（`[shell] narrow_details`）

类型：新能力｜优先级：中

## 现象或需求

ipdrun-35 之后，≤ 960 宽时打开右栏一律按最大化处理（`ui/packages/shell/src/styles/layout.css` 的 `@media (max-width: 960px)`）：
右栏占掉中区的位置，中区留在后面看不见。对一边看右栏的条目、一边在中区和会话说话的应用（IPDrun 的类目 space：右栏是条目，中区是这个类目的会话），
960 宽的笔记本窗口、分屏时就只能来回切。设计稿在 960 宽时是三栏并排：左栏收成 rail，中区和右栏都还够用。

## 建议

profile 加一个开关，hello 带给外壳：

```toml
[shell]
narrow_details = "beside"   # 缺省 "over"（同现在）
```

```ts
interface Hello {
  shell?: { /* … */ narrow_details?: "over" | "beside" };
}
```

- `over`（不写）：同现在。
- `beside`：≤ 960 宽、打开着右栏时照样三栏：`var(--rail-width) minmax(0, 1fr) clamp(260px, 36vw, var(--details-width))`，中区可见、右栏的拖动条照常；
  人点了最大化照旧最大化；< 720 宽两栏都放不下，仍按最大化（同 over）。
- 外壳给 `#frame` 标 `data-narrow="beside"`，规则都在样式表里（不加 JS 的宽度判断）；`App.narrowDetails()` 给需要知道的地方读。

## 兼容性

profile 新键，缺省 `over`，不写的产品（Helm、Grabix、模板）行为不变；`ShellProfile` 是 `deny_unknown_fields`，写了这个键的 profile
拿到旧版 Harnix 会被拒绝读入（新键的通例）。hello 多一个字段，旧外壳不认、忽略。贡献点 API 版本不必升。

## 验收

- Rust：`profile::tests::the_right_column_goes_over_when_narrow_unless_told`——不写是 over，beside 读得出来，写错的值拒绝。
- `ui/packages/shell/test/narrow-details.test.tsx`：hello 不带或 over ⇒ `#frame` 没有 `data-narrow`；beside ⇒ `data-narrow="beside"`、`App.narrowDetails()` 是 beside。
- IPDrun 的 `qa/ui/specs/stage0/t3-t4-layout.spec.ts`：800 宽、profile 写 beside ⇒ 打开右栏后中区与右栏都可见、并排。

## 依赖

ipdrun-35（≤ 960 时右栏可达）。

## 来源

IPDrun R11b 偏离②（stage-R11.md §3）：窄屏外壳把右栏画成盖在中区上的一层，原型 960 时三栏并排；R12 收口（stage-R12.md §1.2）。

## 落地

（待记）
