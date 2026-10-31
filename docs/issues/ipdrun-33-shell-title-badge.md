# ipdrun-33：应用不能给页面标题加待处理数前缀

类型：新能力｜优先级：低

## 需求

页面标题由外壳写：`useBrand` 在品牌名变化时直接 `document.title = name`（`ui/packages/shell/src/brand.ts:41`）。应用想在浏览器页签上显示
「有 N 件事等你」（`(3) 原标题`），只能自己改 `document.title`，再用 `MutationObserver` 盯住 `<head>`，外壳改写标题（切 space、换品牌名）后把前缀补回。
两边抢同一个字符串；同一页里有两份这样的观察者时会互相改写、来回触发。space 的 `badge()`（ipdrun-22）已经有了每个 space 的数，外壳却不汇总到标题上。

## 建议

二选一（都只加不改）：

```ts
interface NestHost {
  // …原有成员不变
  shell: {
    /** 页面标题的数字前缀：n > 0 ⇒ `(n) <标题>`；null 或 0 ⇒ 不带前缀。外壳拼标题时统一加，应用不碰 document.title。 */
    badge(n: number | null): void;
  };
}
```

或者不加接口：外壳把各 space `badge()` 的数字求和（非数字的忽略），自己加到标题前面；由一个开关（如 `brand.titleBadge: true`）打开，缺省关。

## 兼容性

- 纯新增；不调用、不开开关的应用标题照旧。
- 贡献点 API 版本可以不升（`NestHost` 多一个可选成员）。

## 验收

- `ui/packages/shell` 的单元测试：`host.shell.badge(3)` 之后 `document.title === "(3) <品牌名>"`；切 space、品牌名变化之后前缀还在；`badge(null)` 去掉前缀。
- `tests/live/ui/host.spec.ts`：testapp 调一次 `badge(2)`，页签标题带 `(2) `。

## 依赖

无（汇总方案要 ipdrun-22 的 `badge()`，已落地）。

## 来源

IPDrun 首页的「需要你」计数要显示在页签上，现在由 `ui/kit/src/notify.ts` 的 `keepTitlePrefix` 用 `MutationObserver` 补前缀；有了这个原语这段可以删掉。
