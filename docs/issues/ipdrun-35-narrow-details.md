# ipdrun-35：窄于 960 时右栏够不着

类型：缺陷｜优先级：中

## 现象

`ui/packages/shell/src/styles/layout.css` 的 `@media (max-width: 960px)` 把三栏写成 `var(--rail-width) minmax(0, 1fr) 0`：右栏（`#details`）
即使是"打开"（`data-details="open"`）也只有 0 宽。右栏顶上的「最大化」（`data-testid="details-max"`）随之落到中区抬头（`#head`）底下，
点击被中区抬头与「详情栏」按钮挡住（Playwright：`<header id="head"> … intercepts pointer events`）。结果 ≤960 宽时用户打开不了右栏，
类目 space 的列表、条目页都看不到；只有直接改 `data-details-max` 才出来（`#frame[data-details="open"][data-details-max]` 那条规则特异度更高，
最大化本身在窄屏是能用的）。

## 建议

- ≤960 时「详情栏」按钮打开右栏就按最大化处理（右栏占中区的位置，中区留在后面，与 `data-details-max` 同一套栅格）；再点或右栏的关闭 ⇒ 回中区。
- 或者：≤960 时右栏改成盖在中区上的抽屉（`position: fixed`，右侧滑出、宽 `min(100vw - rail, 480px)`），头部的最大化 / 关闭照常可点。

## 兼容性

纯外壳布局，贡献点 API 不变；>960 的行为不变。

## 验收

- `tests/live/ui/layout.spec.ts`：视口 800 宽，点「详情栏」⇒ `#details` 可见且宽度 > 0，里面的按钮可点；关闭 ⇒ 回中区。
- IPDrun 的 `qa/ui/specs/stage0/t3-t4-layout.spec.ts` 的 800 宽一档不再要绕开（现在用 `dispatchEvent("click")` 直接点最大化）。

## 依赖

无。

## 来源

IPDrun R9b 的 T4（窄屏逐个类目核实右栏）：1100 宽（右栏 320）各类目都过，800 宽这一档卡在右栏打不开。

## 落地

落地提交：`a4bc2de`（main，2026-11-01，已推送；版本待下次发布）。≤960 时打开右栏按最大化处理（建议里的第一种）。
