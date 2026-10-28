# LIVE 覆盖表

> 生成的，不要手改：`pnpm --filter @harnix/live-tests coverage`。来源是每个用例的 `@feature`、`@layer`
> 标签；行与说明见 `features.mjs`，行的定义见[设计 §4.3](../../docs/harnix-0.4-design-and-plan.md)。
> `0 · Mx` 是那个里程碑会补上的，`—` 是这一层不适用。

| 功能模块 | id | API | 界面 |
|---|---|---|---|
| 握手与连接 | `connection` | 34 | 2 |
| 授权与设备 | `authz` | 17 | 1 |
| 会话与轮次 | `sessions` | 65 | 15 |
| 无头会话 | `headless` | 4 | 1 |
| 实例层 | `instances` | 24 | — 实例层没有自己的界面；界面上看实例是系统 space 的事（system） |
| 模块契约 | `modules` | 18 | 1 |
| space 与会话绑定 | `spaces` | 7 | 6 |
| 会话 space | `session-space` | 16 | 7 |
| 项目 space | `projects` | — 项目 space 是会话 space 的另一种列法：会话仍属 sessions，API 面由 spaces 与 workspaces 覆盖 | 1 |
| 外壳框架（固定部分、可去掉的内建 space、拖宽、头像） | `shell` | 2 | 22 |
| 系统 space | `system` | 7 | 6 |
| 设置 | `settings` | 12 | 2 |
| 运行期包与界面模块 v2 | `packages` | 11 | 2 |
| 迁移 | `migrations` | 2 | — 迁移没有自己的界面；结果在诊断页（设置 · 诊断） |
| 模块自有链接 | `routes` | 5 | 1 |
| 扩展条目 | `entries` | 5 | 3 |
| 品牌与主题 | `brand` | 2 | 1 |
| 应用构建 | `build` | — 在脚本层检查（check-pins、check-lock、sync-app-deps），不经协议 | — 构建没有界面 |
| 桌面壳 | `desktop` | — 桌面壳不加协议面：webview 加载的就是服务端的页面 | — 桌面壳是模板（templates/desktop）：它不调 IPC 由 check-no-ipc.mjs 在 offline 层查，本机模式由它的 --smoke 查 |
| **合计** | | **231** | **71** |
