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
  { id: "headless", name: "无头会话" },
  {
    id: "instances",
    name: "实例层",
    ui: { na: "实例层没有自己的界面；界面上看实例是系统 space 的事（system）" },
  },
  { id: "modules", name: "模块契约" },
  { id: "spaces", name: "space 与会话绑定" },
  { id: "session-space", name: "会话 space" },
  {
    id: "projects",
    name: "项目 space",
    api: { na: "项目 space 是会话 space 的另一种列法：会话仍属 sessions，API 面由 spaces 与 workspaces 覆盖" },
  },
  { id: "shell", name: "外壳框架（固定部分、可去掉的内建 space、拖宽、头像）" },
  { id: "system", name: "系统 space" },
  { id: "settings", name: "设置" },
  { id: "packages", name: "运行期包与界面模块 v2" },
  { id: "migrations", name: "迁移", ui: { na: "迁移没有自己的界面；结果在诊断页（设置 · 诊断）" } },
  { id: "routes", name: "模块自有链接" },
  { id: "entries", name: "扩展条目" },
  { id: "brand", name: "品牌与主题" },
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
