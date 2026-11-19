# LIVE 覆盖表

> 生成的，不要手改：`pnpm --filter @harnix/live-tests coverage`。来源是每个用例的 `@feature`、`@layer`
> 标签；行（功能模块）与每一行的说明见 `features.mjs`。
> `0 · <何时>` 是还欠着、写明了何时补上的，`—` 是这一层不适用。
> 「进程载体」是扩展包的套件在 `--carrier process` 下再跑一遍的用例数（API 与界面合计）；`—` 是这一行不经扩展包。

| 功能模块 | id | API | 界面 | 进程载体 |
|---|---|---|---|---|
| 握手与连接 | `connection` | 39 | 3 | — |
| 授权与设备 | `authz` | 22 | 2 | — |
| 会话与轮次 | `sessions` | 122 | 48 | — |
| 无头会话 | `headless` | 4 | 1 | — |
| 实例层 | `instances` | 29 | — 实例层没有自己的界面：首页的全局统计只数一数实例与它们的 Agent（home），会话照它所属的 space 打开 | — |
| 扩展契约 | `modules` | 20 | 1 | — |
| space 与会话绑定 | `spaces` | 11 | 6 | — |
| 首页（落地与刷新留在原处、全局统计、待我处理、最近的会话与任务、全部定时、30 天用量） | `home` | — 首页没有自己的协议面：home space 不收会话算在 space 与会话绑定（spaces），它读的方法各在自己的行 | 8 | — |
| 会话 space（含右栏常驻的上下文页签与后台任务的输出页签） | `session-space` | 37 | 39 | — |
| 项目 space（项目树、项目菜单、项目信任、项目里的任务、项目概况与待我处理） | `projects` | 3 | 17 | — |
| 外壳框架（固定部分、可去掉的内建 space、拖宽、头像） | `shell` | 3 | 44 | — |
| 设置（含诊断与放进诊断的区块） | `settings` | 44 | 28 | — |
| Agix 包（插件）与它们的界面模块 | `plugins` | 13 | 2 | — |
| 迁移 | `migrations` | 2 | — 迁移没有自己的界面；结果在诊断页（设置 · 诊断） | — |
| 扩展的 HTTP 路由 | `routes` | 5 | 1 | — |
| 扩展条目 | `entries` | 5 | 3 | — |
| 品牌与主题 | `brand` | 4 | 1 | — |
| 流程（harnix-ext-flow 与流程 space，xops-01、xops-02） | `flow` | 45 | 27 | — |
| 定时（harnix-ext-schedule：时间与时区、三种目标、错过与并发、回执、模型建的定时） | `schedule` | 6 | 3 | — |
| 扩展包（登记与撤下、安装与卸载、启停、工具、agent 包、界面模块、隔离 iframe 里的界面与外壳的桥、MCP 端点、WASM 后端、扩展进程、诊断里的运行状况、会话看得到的 skill 与 hook、诊断里的扩展日志） | `packages` | 69 | 19 | 69 |
| 扩展包商店（签名与副署、目录、按名字安装与升级、披露与同意、设置 · 扩展包） | `packages-store` | 11 | 5 | 16 |
| 扩展包的声明式行内点与管理界面（工具行模板、流块卡片、命令候选、侧栏分组、披露弹窗、清空数据、左栏标记） | `packages-inline` | 4 | 6 | 10 |
| 扩展包后端的闲置休眠（on-demand、冷启动、并发唤醒只起一个实例、订阅与定时器保持常驻、诊断里的运行状况） | `packages-idle` | 6 | 1 | 7 |
| 应用构建 | `build` | — 在脚本层检查（check-pins、check-lock、sync-app-deps），不经协议 | — 构建没有界面 | — |
| 桌面壳 | `desktop` | — 桌面壳不加协议面：webview 加载的就是服务端的页面 | — 桌面壳是模板（templates/desktop）：它不调 IPC 由 check-no-ipc.mjs 在 offline 层查，本机模式由它的 --smoke 查 | — |
| **合计** | | **504** | **265** | **102** |

## 已知缺口（`@gap`）

> 这些用例把已知的错误行为照原样钉住，不算覆盖。对应的修复合入后，改成正确的断言、去掉 `@gap`。

| 功能模块 | 层 | 用例 | 等什么修 |
|---|---|---|---|
| `authz` | api | api/audit.spec.ts › audit › deleting a session takes its audit entry with it | a durable sink for harnix.session.delete that is not the deleted transcript (crates/assembly/src/audit_log.rs) |
