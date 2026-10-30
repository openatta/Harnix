# grabix-02：space 不能定右栏的缺省宽度（`detailsWidth`）

类型：新能力｜优先级：低

## 需求

右栏宽度的三个数是外壳常量：`WIDTHS.details = [320, 360, 480]`（最小 / 缺省 / 最大，`ui/packages/shell/src/app.ts`）。人拖过的宽度存在
`localStorage["harnix.widths"]`，全局、不分 space。space 没有任何办法说「我的右栏要宽一些」：右栏放宽表格、预览的 space，
每次进来都是 360，要人自己拖。又因为最大是 480，宽表也撑不开。

## 建议

```ts
export interface SpaceContribution {
  // …原有成员不变
  /** 该 space 在前台时右栏的缺省宽度（px）。人拖过的宽度优先。夹在 320–720。 */
  detailsWidth?: number;
}
```

- 取值：人没拖过右栏时，用 `detailsWidth`，夹在 [320, 720] 并取整；没有该字段仍是 360。
- 人拖过的宽度优先；双击拖柄（重置）回到该 space 的 `detailsWidth`，不是 360。
- 最大值：外壳常量 480 不变；**声明了 `detailsWidth` 的 space 在前台时，右栏的上限放宽到 720**（`DETAILS_WIDE_MAX`），不然缺省值超过 480 就没有意义，
  人也拖不出比缺省更宽的。别的 space 上限仍是 480；人在宽 space 里拖到 650，进别的 space 时被压到 480，存的值不丢。
- 「人拖过」要分得清：现在 `setWidth` 把两栏的宽度一起存，拖一下左栏也会把右栏的 360 存进去，等于冻住了缺省。改为只存人真正拖过的那一栏
  （`ShellState.dragged`），`localStorage["harnix.widths"]` 的格式不变（仍是 `{side?, details?}`，只是少了没拖过的键）。
- 只读 `detailsWidth` 当前值，不监听；space 变了就 `host.redraw()`（ipdrun-19）。

## 兼容性

- 不给 `detailsWidth` 的 space：宽度、上限、拖动与重置都不变。
- 旧版本存下的 `harnix.widths` 里若带着没拖过的 `details: 360`，会被当作人拖过的值，`detailsWidth` 对这个浏览器不生效，直到人双击重置一次；
  新装的浏览器不受影响。
- 贡献点 API 版本：可选字段，可以不升。
- Helm、hello-app 不受影响。

## 验收

- `tests/live/ui/host.spec.ts`：testapp 注册带与不带 `detailsWidth` 的 space，
  - 不带：`#frame` 的 `--details-width` 是 360px；带 560：进入即 560px；带 5000：夹到 720px；
  - 拖到 650：宽 space 里是 650px，进不带的 space 压到 480px；在不带的 space 里拖 900：480px；
  - 重置后各回各的缺省（360 / 560）；只拖左栏不冻住右栏的缺省。

## 依赖

无。

## 来源

Grabix 的数据页详情是宽表格，右栏 360 放不下，希望缺省约 480 以上；原先只能记为限制、请人拖宽。

## 实现记录

外壳：

- `SpaceContribution.detailsWidth?: number`（`ui/packages/react/src/contrib.ts`）。
- `app.ts`：`ShellState.dragged`（人拖过的栏）、`detailsWidthOf(state, space)`、`DETAILS_WIDE_MAX = 720`；`setWidth` 只存拖过的栏，声明了 `detailsWidth` 的前台 space 里右栏上限为 720，`null` 删掉该栏的记录。
- `Shell.tsx`：`--details-width` 与拖动起点都取 `detailsWidthOf`。
- 测试：`tests/live/ui/host.spec.ts` 的「detailsWidth: …」。
