# ipdrun-46：产品关不掉整个外壳的「新会话」（`[shell] new_session`）

类型：新能力｜优先级：中

## 现象或需求

左栏顶上的「新会话」按钮与 ⌘K（`ui/packages/shell/src/Sidebar.tsx:88`、`ui/packages/shell/src/Shell.tsx:148`）只能由前台 space
的 `newSession: false`（grabix-01）一个一个关掉。一个产品若根本不让人从外壳建会话（会话都由它自己的 space 按业务建），
就得在每个 space 上都写 `newSession: false`，外壳自带的 space（会话、项目、系统）它又写不了；⌘K 也一直被外壳占着
（`preventDefault`），应用想把这个键给自己用也拿不到。

## 建议

profile 加一个开关，hello 带给外壳：

```toml
[shell]
new_session = false   # 缺省 true
```

```ts
interface Hello {
  shell?: { /* … */ new_session?: boolean };
}
```

- `false`：哪个 space 在前台，左栏都不画「新会话」按钮（连同它下面的分隔线）；⌘K 外壳不管（不 `preventDefault`、不新建），留给页面。
- 不写或 `true`：同现在。space 自己的 `newSession: false` 照旧叠加在上面。
- 只是界面：中枢照旧收 `session.create`。

## 兼容性

profile 新键，缺省 `true`，不写它的产品（Helm、Grabix、模板）行为不变；`ShellProfile` 是 `deny_unknown_fields`，
所以写了这个键的 profile 拿到旧版 Harnix 会被拒绝读入（新键的通例）。hello 多一个字段，旧外壳不认、忽略。贡献点 API 版本不必升。

## 验收

- Rust：`profile::tests::the_shell_starts_new_sessions_unless_told`——不写是 `true`，`false` 读得出来，`ShellProfile::default()` 是 `true`。
- `ui/packages/shell/test/new-session-switch.test.tsx`：
  - hello 不带或带 `true`：有 `#new` 与它后面的 `.sep`，⌘K 被外壳接住。
  - `false`：没有 `#new`、没有那条分隔线，⌘K 不被 `preventDefault`、当前会话不变。
  - `true` 时 space 的 `newSession: false` 照旧藏掉按钮。

## 依赖

—（与 grabix-01 的 space 级 `newSession` 叠加）

## 来源

IPDrun R11 「项目」space：stage-R11.md §1.10。IPDrun 的会话都由类目与「项目」space 按业务建，外壳的「新会话」在哪个 space 都不该出现。

## 落地

（待填）
