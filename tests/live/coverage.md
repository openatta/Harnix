# LIVE 覆盖表

> 生成的，不要手改：`pnpm --filter @harnix/live-tests coverage`。来源是每个用例的 `@feature`、`@layer`
> 标签；行与说明见 `features.mjs`，行的定义见[设计 §4.3](../../docs/harnix-0.4-design-and-plan.md)。
> `0 · Mx` 是那个里程碑会补上的，`—` 是这一层不适用。

| 功能模块 | id | API | 界面 |
|---|---|---|---|
| 握手与连接 | `connection` | 30 | 0 · M4 |
| 授权与设备 | `authz` | 17 | 0 · M4 |
| 会话与轮次 | `sessions` | 53 | 5 |
| 无头会话 | `headless` | 0 · M3 | 0 · M3 |
| 实例层 | `instances` | 0 · M3 | — 实例层没有自己的界面；界面上看实例是系统 space 的事（system） |
| 模块契约 | `modules` | 0 · M2 | 0 · M5 |
| space 与会话绑定 | `spaces` | 0 · M5 | 0 · M5 |
| 会话 space | `session-space` | 16 | 5 |
| HOME | `home` | 0 · M5 | 0 · M5 |
| 系统 space | `system` | 0 · M6 | 0 · M6 |
| 设置 | `settings` | 7 | 0 · M4 |
| 运行期包 | `packages` | 8 | 0 · M8 |
| 迁移 | `migrations` | 0 · M2 | 0 · M2 |
| 模块自有链接 | `routes` | 0 · M2 | — 自有链接由模块自己的界面用，没有 Harnix 的界面 |
| 扩展条目 | `entries` | 0 · M7 | 0 · M7 |
| 品牌与主题 | `brand` | 0 · M8 | 0 · M8 |
| 应用构建 | `build` | — 在脚本层检查（check-pins、check-lock、sync-app-deps），不经协议 | — 构建没有界面 |
| 桌面壳 | `desktop` | — 桌面壳不加协议面：webview 加载的就是服务端的页面 | 0 · M9 |
| **合计** | | **131** | **10** |
