# ipdrun-49：左栏品牌下面没有地方放应用的上下文（`host.shell.context`）

类型：新能力｜优先级：低

## 现象或需求

左栏顶上只有品牌（图标与产品名，`ui/packages/shell/src/Sidebar.tsx` 的 `.brand`）。应用若有一个贯穿所有 space 的上下文
（此刻在哪个工作对象上），没有固定的位置显示：放进某个 space 的 `sidebar()` 就只在那个 space 里看得到，
放进 `headerActions` 又挤在中区标题栏。

## 建议

```ts
interface NestHost {
  shell: {
    /** 品牌下面一行应用的上下文；null ⇒ 不画。 */
    context(c: { label: string; title?: string; tone?: "active" | "muted"; onClick?: () => void } | null): void;
  };
}
```

- 外壳记在自己的状态里，左栏在品牌块下面画一行（`data-testid="shell-context"`、`data-tone`）：
  `tone: "active"` 前面一个小圆点，`"muted"` 是浅色字；给了 `title` 就是悬停提示；给了 `onClick` 这一行是按钮，否则是普通文字。
- `null`（或 `label` 为空）⇒ 不画。不跟 space 走：换 space 它还在。

## 兼容性

`NestHost.shell` 上的新成员；不调用的应用左栏同现在。自己实现 `NestHost` 的测试替身要补这个成员（类型检查会指出来）。
贡献点 API 版本不必升。

## 验收

`ui/packages/shell/test/shell-context.test.tsx`：

- 不调用：没有 `shell-context`。
- `context({label, title, tone: "active"})`：品牌下面一行，文字是 `label`，`title`、`data-tone="active"`、有圆点，不是按钮。
- 带 `onClick`：是按钮，点了调到它；`tone: "muted"` 没有圆点。
- `context(null)` ⇒ 去掉；换 space 时还在。

## 依赖

建在 ipdrun-33 的 `host.shell` 上。

## 来源

IPDrun R11 「项目」space：stage-R11.md §1.10。左栏品牌下面显示当前项目名，没有打开的项目时是浅色的「没有打开的项目」，点一下去「项目」space。

## 落地

（待填）
