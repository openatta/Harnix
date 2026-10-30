# grabix-01：space 不能关掉「新会话」（`newSession`）

类型：新能力｜优先级：中

## 需求

有的 space 不是通用对话：它的会话由应用自己建（经自己的方法），不该让人在界面上随手新建。外壳现在无条件给出三处入口，space 一处也关不掉：

- 左栏的「新会话」按钮 `#new`（`ui/packages/shell/src/Sidebar.tsx`，`id="new"`），⌘K 就是点它（`Shell.tsx` 的 `useShortcuts`）；
- 空中区（`Hero`，`Shell.tsx`）里会建会话的输入框 `FirstMessage` 与建议条；`center: "minimized"` 的 `Overview` 里的提问框（`variant="ask"`）也是同一个组件，输入即建会话；
- 没有前台会话时抬头的标题写「新会话」（`Header`）。

profile 的 `[shell]` 只有 `spaces`、`model_switch`，`SpaceContribution` 与宿主方法都没有相关开关。space 只能用 `readOnly` 拦已绑定的会话，拦不到「还没有会话」的空中区。

## 建议

```ts
export interface SpaceContribution {
  // …原有成员不变
  /** false：该 space 在前台时，外壳不画「新会话」按钮（⌘K 随之无效）、不画会建会话的输入框与建议条、
   *  抬头不写「新会话」。缺省 true。 */
  newSession?: boolean;
}
```

- 只看前台 space 的这个字段，每次画时读，不另存状态。
- 空中区：不画 `FirstMessage` 与建议条，改画一行静态说明，文字取 `placeholder()`（没有就空）；`subtitle()` 照旧在上面。
- 总览（`center: "minimized"`）：不画提问框。
- 抬头：没有前台会话时写 space 的 `title()`，不写「新会话」。
- 已存在的会话照常打开、照常可写（要只读用 `readOnly`）。
- 只管界面：不改 Rust、协议、profile；hub 仍接受 `session.create`，这是防误操作，不是权限边界。
- 其它入口不在本条范围：项目 space 里「在这个项目里新建会话」的对话框、系统 space 自己的按钮，由各自 space 决定。

## 兼容性

- 不给 `newSession` 的 space：画面与行为完全不变。
- 贡献点 API 版本：可选字段，可以不升。
- 注册时不做形状校验的新规则；非布尔值只有 `false` 才生效。
- Helm、hello-app 不受影响。

## 验收

- `tests/live/ui/host.spec.ts`：testapp 里注册 `newSession: false` 的 space，前台时
  - 没有 `#new`，按 ⌘K / Ctrl+K 后不出现 `first-message`，也没有会话被建；
  - 空中区没有 `first-message` 与建议条，有 `no-new-session` 的静态说明；抬头是 space 的标题；
  - 已有会话能打开，`conv-input` 在；
  - `center: "minimized"` 的此类 space，总览里没有 `ask`。
- 切到不带该字段的 space：`#new`、`first-message` 都回来。

## 依赖

无。

## 来源

Grabix 不是通用聊天工具，会话只由应用的任务流程建，不要「新建会话」入口；在此之前只能用 `readOnly` 过渡。

## 实现记录

落地提交：`0debd3e`（main，2026-10-30，已推送；版本待下次发布）。

外壳：

- `SpaceContribution.newSession?: boolean`（`ui/packages/react/src/contrib.ts`）。
- `Sidebar.tsx`：前台 space `newSession === false` 时不画 `#new` 及其下的分隔线，⌘K 因找不到 `#new` 而无事发生（`useShortcuts` 未改）。
- `Shell.tsx`：`Hero` 以 `no-new-session` 的静态说明代替 `FirstMessage` 与建议条；`Overview` 不画 `ask`；抬头无会话时写 space 的 `title()`。
- 测试：`tests/live/ui/host.spec.ts` 的「newSession: false: …」。
