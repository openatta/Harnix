# LIVE 覆盖表

> 生成的，不要手改：`pnpm --filter @harnix/live-tests coverage`。来源是每个用例的 `@feature`、`@layer`
> 标签；行（功能模块）与每一行的说明见 `features.mjs`。
> `0 · <何时>` 是还欠着、写明了何时补上的，`—` 是这一层不适用。
> 「进程载体」是扩展包的套件在 `--carrier process` 下再跑一遍的用例数（API 与界面合计）；`—` 是这一行不经扩展包。

| 功能模块 | id | API | 界面 | 进程载体 |
|---|---|---|---|---|
| 握手与连接 | `connection` | 39 | 3 | — |
| 授权与设备 | `authz` | 22 | 2 | — |
| 会话与轮次 | `sessions` | 92 | 33 | — |
| 无头会话 | `headless` | 4 | 1 | — |
| 实例层 | `instances` | 28 | — 实例层没有自己的界面；界面上看实例是系统 space 的事（system） | — |
| 扩展契约 | `modules` | 20 | 1 | — |
| space 与会话绑定 | `spaces` | 10 | 6 | — |
| 会话 space | `session-space` | 19 | 21 | — |
| 项目 space（项目树、项目菜单、项目信任） | `projects` | 3 | 7 | — |
| 外壳框架（固定部分、可去掉的内建 space、拖宽、头像） | `shell` | 2 | 43 | — |
| 系统 space | `system` | 16 | 29 | — |
| 设置 | `settings` | 18 | 10 | — |
| Agix 包（插件）与它们的界面模块 | `plugins` | 13 | 2 | — |
| 迁移 | `migrations` | 2 | — 迁移没有自己的界面；结果在诊断页（设置 · 诊断） | — |
| 扩展的 HTTP 路由 | `routes` | 5 | 1 | — |
| 扩展条目 | `entries` | 5 | 3 | — |
| 品牌与主题 | `brand` | 2 | 1 | — |
| 流程（harnix-ext-flow，xops-01、xops-02） | `flow` | 30 | 28 | — |
| 扩展包（登记与撤下、安装与卸载、启停、工具、agent 包、界面模块、隔离 iframe 里的界面与外壳的桥、MCP 端点、WASM 后端、扩展进程、系统 space 的扩展包页签） | `packages` | 64 | 18 | 63 |
| 扩展包商店（签名与副署、目录、按名字安装与升级、披露与同意、设置 · 扩展包） | `packages-store` | 11 | 5 | 16 |
| 扩展包的声明式行内点与管理界面（工具行模板、流块卡片、命令候选、侧栏分组、披露弹窗、清空数据、左栏标记） | `packages-inline` | 4 | 6 | 10 |
| 扩展包后端的闲置休眠（on-demand、冷启动、并发唤醒只起一个实例、订阅与定时器保持常驻、系统 space 的扩展包页签） | `packages-idle` | 6 | 1 | 7 |
| 应用构建 | `build` | — 在脚本层检查（check-pins、check-lock、sync-app-deps），不经协议 | — 构建没有界面 | — |
| 桌面壳 | `desktop` | — 桌面壳不加协议面：webview 加载的就是服务端的页面 | — 桌面壳是模板（templates/desktop）：它不调 IPC 由 check-no-ipc.mjs 在 offline 层查，本机模式由它的 --smoke 查 | — |
| **合计** | | **415** | **221** | **96** |

## 已知缺口（`@gap`）

> 这些用例把已知的错误行为照原样钉住，不算覆盖。对应的修复合入后，改成正确的断言、去掉 `@gap`。

| 功能模块 | 层 | 用例 | 等什么修 |
|---|---|---|---|
| `authz` | api | api/audit.spec.ts › audit › deleting a session takes its audit entry with it | a durable sink for harnix.session.delete that is not the deleted transcript (crates/assembly/src/audit_log.rs) |
