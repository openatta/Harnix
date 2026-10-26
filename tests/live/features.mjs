// The functional modules, as rows (design §4.3), and what each layer owes.
//
// Every test carries `@feature:<id>` and `@layer:api|ui`. `coverage.mjs`
// counts them into `coverage.md`; a row that is zero on a layer must say why
// here — `na` (this layer does not apply, and the reason) or `pending` (the
// milestone that brings it). Anything else fails `scripts/check.sh`.
//
// When a milestone lands its tests, its `pending` goes — a stale `pending`
// over a non-zero count also fails, so the table cannot keep promising work
// that is already done.

export const FEATURES = [
  { id: "connection", name: "握手与连接" },
  { id: "authz", name: "授权与设备" },
  { id: "sessions", name: "会话与轮次" },
  { id: "headless", name: "无头会话", ui: { pending: "M6" } },
  {
    id: "instances",
    name: "实例层",
    ui: { na: "实例层没有自己的界面；界面上看实例是系统 space 的事（system）" },
  },
  { id: "modules", name: "模块契约" },
  { id: "spaces", name: "space 与会话绑定" },
  { id: "session-space", name: "会话 space" },
  {
    id: "home",
    name: "HOME",
    api: { na: "HOME 是界面的一页；它的会话是 home space 的会话，API 面由 spaces 覆盖" },
  },
  { id: "system", name: "系统 space", api: { pending: "M6" }, ui: { pending: "M6" } },
  { id: "settings", name: "设置" },
  { id: "packages", name: "运行期包", ui: { pending: "M8" } },
  { id: "migrations", name: "迁移", ui: { na: "迁移没有自己的界面；结果在诊断页（设置 · 诊断）" } },
  { id: "routes", name: "模块自有链接", ui: { na: "自有链接由模块自己的界面用，没有 Harnix 的界面" } },
  { id: "entries", name: "扩展条目", api: { pending: "M7" }, ui: { pending: "M7" } },
  { id: "brand", name: "品牌与主题", api: { pending: "M8" }, ui: { pending: "M8" } },
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
    ui: { pending: "M9" },
  },
];
