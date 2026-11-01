# ipdrun-47：左栏的 space 列表不能分组（`SpaceContribution.group`）

类型：新能力｜优先级：低

## 现象或需求

左栏中段的 space 列表（`ui/packages/shell/src/Sidebar.tsx` 的 `SpaceList`）按 `order` 一溜排下来，中间画不了分隔线。
一个产品的 space 若分两类（例如「管全局的」与「做业务的」），只能靠 `order` 挨在一起，看不出是两组；
`place: "bottom"` 能把一个 space 挪到底部，但那是另一种位置，不是同一列表里分组。

## 建议

```ts
interface SpaceContribution {
  /** 左栏列表里的组名：相邻两个 space 的组不同，中间画一条分隔线。不写算 ""。 */
  group?: string;
}
```

- 列表仍按 `order` 排；画的时候相邻两项的 `group`（不写当 `""`）不同，中间画一条分隔线
  （`<div className="sep" role="separator" data-testid="space-sep" />`）。
- 分组不重排：同组的 space 要挨在一起，由 `order` 保证。
- 只管顶部列表；`place: "bottom"` 的与系统入口不在其内。

## 兼容性

可选字段；都不写 ⇒ 都是 `""`，一条分隔线也不画，同现在。贡献点 API 版本不必升。

## 验收

`ui/packages/shell/test/nav-group-disabled.test.tsx`：

- 不写 `group` 的几个 space：没有 `space-sep`。
- `a`、`b` 不写，`c`、`d` 写 `"work"`：恰好一条分隔线，在 `b` 与 `c` 之间。

## 依赖

—

## 来源

IPDrun R11 「项目」space：stage-R11.md §1.10。左栏上面是「项目」（管全局），下面是项目内的类目，两组之间要一条线。

## 落地

落地提交：`ebe867d`（main，2026-11-01，未推送；版本待下次发布）。
`SpaceContribution.group?: string`（`ui/packages/react/src/contrib.ts`）；`Sidebar.tsx` 的 `SpaceList` 在组不同的相邻两项之间画
`<div className="sep" role="separator" data-testid="space-sep" />`。
测试：`ui/packages/shell/test/nav-group-disabled.test.tsx`（space groups，2）。
