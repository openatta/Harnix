// The functional modules, as rows (testing.md §7), and what each layer owes.
//
// Every test carries `@feature:<id>` and `@layer:api|ui`. `coverage.mjs`
// counts them into `coverage.md`; a row that is zero on a layer must say why
// here — `na` (this layer does not apply, and the reason) or `pending` (the
// milestone that brings it). Anything else fails `scripts/check.sh`.
//
// When a milestone lands its tests, its `pending` goes — a stale `pending`
// over a non-zero count also fails, so the table cannot keep promising work
// that is already done.
//
// A row marked `carrier: true` is about extension packages, which run on either carrier: its
// tests in `CARRIER_SUITES` run once more with each package in a process of its own
// (`pnpm live --carrier process`), and the table counts those in a column of its own. Zero there
// fails like zero on a layer; a row not marked that has tests there fails too.

export const FEATURES = [
  { id: "connection", name: "握手与连接" },
  { id: "authz", name: "授权与设备" },
  { id: "sessions", name: "会话与轮次" },
  { id: "headless", name: "无头会话" },
  {
    id: "instances",
    name: "实例层",
    ui: { na: "实例层没有自己的界面；界面上看实例是系统 space 的事（system）" },
  },
  { id: "modules", name: "扩展契约" },
  { id: "spaces", name: "space 与会话绑定" },
  { id: "session-space", name: "会话 space" },
  { id: "projects", name: "项目 space（项目树、项目菜单、项目信任、项目里的任务、项目概况与待我处理）" },
  { id: "shell", name: "外壳框架（固定部分、可去掉的内建 space、拖宽、头像）" },
  { id: "system", name: "系统 space" },
  { id: "settings", name: "设置" },
  { id: "plugins", name: "Agix 包（插件）与它们的界面模块" },
  { id: "migrations", name: "迁移", ui: { na: "迁移没有自己的界面；结果在诊断页（设置 · 诊断）" } },
  { id: "routes", name: "扩展的 HTTP 路由" },
  { id: "entries", name: "扩展条目" },
  { id: "brand", name: "品牌与主题" },
  { id: "flow", name: "流程（harnix-ext-flow，xops-01、xops-02）" },
  { id: "packages", carrier: true, name: "扩展包（登记与撤下、安装与卸载、启停、工具、agent 包、界面模块、隔离 iframe 里的界面与外壳的桥、MCP 端点、WASM 后端、扩展进程、系统 space 的扩展包页签、会话看得到的 skill 与 hook、扩展日志）" },
  { id: "packages-store", carrier: true, name: "扩展包商店（签名与副署、目录、按名字安装与升级、披露与同意、设置 · 扩展包）" },
  { id: "packages-inline", carrier: true, name: "扩展包的声明式行内点与管理界面（工具行模板、流块卡片、命令候选、侧栏分组、披露弹窗、清空数据、左栏标记）" },
  { id: "packages-idle", carrier: true, name: "扩展包后端的闲置休眠（on-demand、冷启动、并发唤醒只起一个实例、订阅与定时器保持常驻、系统 space 的扩展包页签）" },
  {
    id: "build",
    name: "应用构建",
    api: { na: "在脚本层检查（check-pins、check-lock、sync-app-deps），不经协议" },
    ui: { na: "构建没有界面" },
  },
  {
    id: "desktop",
    name: "桌面壳",
    api: { na: "桌面壳不加协议面：webview 加载的就是服务端的页面" },
    ui: { na: "桌面壳是模板（templates/desktop）：它不调 IPC 由 check-no-ipc.mjs 在 offline 层查，本机模式由它的 --smoke 查" },
  },
];

// The suites that hold on both carriers, as globs under `tests/live`: `--carrier process` runs
// these and nothing else (`playwright.config.ts`). A suite about one carrier pins it in its own
// profile and is left out (`CARRIER_PINNED`): it runs in every run already.
export const CARRIER_SUITES = ["api/packages*.spec.ts", "ui/packages*.spec.ts"];
export const CARRIER_PINNED = ["api/packages-process.spec.ts"];

// Methods a client may call (`harnix.reachable` on the Harnix app) that no API test calls,
// each with why. Under `HARNIX_COVERAGE_GATE=1` (what `scripts/check.sh` sets for a whole `api`
// run) the global teardown fails on a reachable method that is neither called nor listed here,
// and on an entry here that a test now calls — the same two rules as the rows above.
export const UNCALLED = {};
