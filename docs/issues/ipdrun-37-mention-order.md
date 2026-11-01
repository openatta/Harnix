# ipdrun-37：`@` 菜单里应用贡献的候选被文件挤掉

类型：缺陷｜优先级：中

## 现象或需求

`@` 菜单的候选由外壳拼：自己的（项目文件、`@session:`）在前，`trigger: "@"` 的 `command` 贡献在后，截到 20 条
（`ui/packages/shell/src/Shell.tsx:887` `withContributed`）；输入框再只显示前 8 条（`ui/packages/react/src/chat/ChatComposer.tsx:266`）。
项目里匹配的文件一多，贡献的候选就一条也看不到。对某些 space，`@` 主要是引用它自己的东西，文件反而是次要的。

## 建议

```ts
interface SpaceContribution {
  /** 本 space 在前台时 `@` 菜单里谁的候选在前：外壳自己的（"files"，缺省）或贡献的（"contributed"）。 */
  mention?: { first?: "contributed" | "files" };
}
```

`withContributed` 按前台 space 的 `mention.first` 决定拼接顺序，其余（上限、超时、失败计数）不变。

## 兼容性

可选字段；不写或写 `"files"` 与现在一样。贡献点 API 版本不必升。

## 验收

`ui/packages/shell/test/mention-order.test.ts`：25 个文件 + 1 个贡献的候选——缺省时截断后没有贡献的那条；`first: "files"` 文件在前；
`first: "contributed"` 时贡献的排第一、文件随后，总数仍是 20。

## 依赖

无（建在 ipdrun-27 的 `trigger: "@"` 上）。

## 来源

IPDrun 的类目对话里 `@` 引用条目（需求、规格…）；仓库文件一多，条目候选就被挤出前 8 条。

## 落地

外壳：`withContributed`（`ui/packages/shell/src/Shell.tsx`，导出以便测试）读前台 space 的 `mention.first`。
