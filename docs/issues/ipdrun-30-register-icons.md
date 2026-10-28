# ipdrun-30：space 的图标只能从外壳自带的那几十个里挑

类型：新能力｜优先级：低

## 需求

`SpaceContribution.icon` 是一个图标名（`ui/packages/react/src/contrib.ts:78-79`），解析在外壳的 `ICON` 表里（`ui/packages/shell/src/icons.ts:9` 起，
三十来个通用图标；`Glyph` 组件按名字取，`ui/packages/shell/src/glyph.tsx`）。应用加不了自己的图标，只能挑一个最接近的。
`tokens`、`strings` 都能按 owner 注册（`contrib.ts:176`、`:208`），图标不能。

## 建议

注册表加一个与 `tokens`、`strings` 并列的方法：

```ts
class Registry {
  // …原有成员不变
  /** 应用自己的图标：名字 → SVG 的内部（不含 <svg>），画在 24 的网格上、1.7 的描边，与外壳的图标同一套外框。 */
  icons(icons: Record<string, string>, owner: string): void;
}
```

- 名字按 owner 加前缀（`<owner>.<name>`）避免撞名，space 的 `icon` 写全名；不带前缀的名字仍按外壳的表解析。
- 外壳用 `icons.ts` 的 `wrap()` 包上外框，保证尺寸、描边、`currentColor` 与内建图标一致。
- **安全**：只接受 SVG 的形状元素（`path`、`circle`、`rect`、`line`、`polyline`、`polygon`、`ellipse`、`g`）和它们的几何、描边属性；`<script>`、
  `on*` 属性、`href`、`style`、外部引用一律剥掉或拒收，拒收进诊断页。界面模块是页面级信任，这一条是**防误**。
- 找不到的图标名画一个占位并在诊断里记一条（与现在未知名字的处理一致即可）。

## 兼容性

- 纯新增；`icon` 仍可以写外壳的名字。
- 贡献点 API 版本：注册表多一个方法，可以不升。
- 运行期包的界面模块也能用（与 `tokens`、`strings` 同权）。Helm、hello-app 不受影响。

## 验收

- `ui/packages/react/test/contrib.test.ts`：`icons({ ship: "<path d='M3 3h18'/>" }, "testapp")` 之后，`testapp.ship` 可解析；带 `<script>` 或 `onload` 的被拒、理由进 `refusals()`。
- `tests/live/ui/interface.spec.ts`：testapp 的 space 用 `icon: "testapp.ship"`，左栏入口的 SVG 里有那条 `path`，尺寸与相邻内建图标相同。

## 依赖

无。

## 来源

IPDrun 原型里各 space 的图标（剪贴板、叠放的平面、git 合并、流程线…）大多不在外壳的表里，现在取了最接近的。

## 实现记录

外壳（F3b）：

- `Registry.icons(icons, owner)` 与 `Registry.icon(name)`（`ui/packages/react/src/contrib.ts`）：名字是 `<owner>.<name>`（`name` 只许字母、数字、`-`、`_`）；
  内容用 `DOMParser` 按 SVG 读，只收 `path`、`circle`、`rect`、`line`、`polyline`、`polygon`、`ellipse`、`g` 与几何、描边属性，按读到的节点重新拼出来存；
  脚本、`on*`、`href`、`style`、`url(` 之类整个图标拒收，理由进 `refusals()`（`point: "icons"`）。运行期包拿到的签名注册表也有 `icons`（签成包名）。
- 外壳 `icons.ts` 的 `glyphHtml` 先查自带的表，再经 `iconsFrom(...)`（`main.tsx` 接上注册表）查应用的，用同一个 `wrap()` 外框；都没有时照旧不画（与现在未知名字一致，
  没加占位与诊断）。
- 测试：`ui/packages/react/test/contrib.test.ts` 的「takes an application's icons under its own name, and refuses a script, a handler, a link」；
  `tests/live/ui/host.spec.ts` 的「icons: a space's own icon, by its owner's name, drawn in the shell's frame at the size of the others」。
  偏离：图标在页面里注册（owner `test`），不改 testapp。
