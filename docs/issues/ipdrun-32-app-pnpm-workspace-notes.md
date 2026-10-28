# ipdrun-32：外部应用的两个 pnpm workspace：模板少一行，文档少两句

类型：文档｜优先级：低

## 现象

SDK §3 的「两个 pnpm workspace，互不相交」（`docs/harnix-sdk.md:90-93`）在外部应用上走得通：`harnix/` 自己装、外壳在那里构建，
应用根的 workspace 只收 `spaces/*/ui` 与测试包，`@harnix/react` 用 `link:` 引进来。照着做的时候有三处没写到：

1. **模板的构建脚本少一行。** `templates/app/scripts/build.sh:12-13` 只有 `pnpm --dir harnix install --frozen-lockfile`，没有应用根的
   `pnpm install`。有自己 workspace 的应用（SDK 推荐的做法）第一次构建时，应用的第三方依赖没装，外壳构建在解析 space 的 import 时失败。
2. **类型检查。** `@harnix/react` 的 `exports` 指向 `.ts` 源码，应用的 `tsc` 会一起检查它：应用的 tsconfig 要有 `allowImportingTsExtensions`
   （继承 `harnix/tsconfig.base.json` 就有；继承自己的 base 时要自己加）。
3. **vitest 里两份 React。** 在应用的 vitest 里，`link:` 进来的 `@harnix/react` 从 `harnix/` 那边解析 `react`，组件测试里出现两份 React，
   hook 报错。要在应用的 vitest 配置里写 `resolve: { dedupe: ["react", "react-dom"] }`——与外壳构建时 Vite 的去重是同一件事，方向相反。

## 建议

- `templates/app/scripts/build.sh` 在 `pnpm --dir harnix install` 之后加：
  ```bash
  # The app's own workspace (spaces/*/ui, tests), when it has one.
  if [ -f pnpm-workspace.yaml ]; then pnpm install --frozen-lockfile; fi
  ```
- SDK §3 那一段后面补第 2、3 条各一句；§12（测试）提一句 vitest 的 `dedupe`。

## 兼容性

- 模板脚本只在应用根有 `pnpm-workspace.yaml` 时多跑一次安装；没有自己 workspace 的应用不变。
- 文档改动不影响任何代码。已经从模板建好的应用（Helm 等）不受影响，下次同步模板时可以带上。

## 验收

- `tests/app-template.mjs`：用模板建一个带 `pnpm-workspace.yaml` 和一个依赖第三方包的 space 的应用，`scripts/build.sh` 一次通过。
- 不带 `pnpm-workspace.yaml` 的模板应用照旧通过（回归）。

## 依赖

无。

## 来源

IPDrun 迁到 Harnix 时按 SDK §3 用两个 workspace，走通了；这三处是当时补的（构建脚本里的根 `pnpm install`、tsconfig、vitest 的 `dedupe`）。

## 实现记录

- `templates/app/scripts/build.sh`：`pnpm --dir harnix install` 之后、外壳构建之前，应用根有 `pnpm-workspace.yaml` 时 `pnpm install --frozen-lockfile`。
- SDK 指南 §3「两个 pnpm workspace」后补三条（两边都要装、`allowImportingTsExtensions`、两份 React 指向 §12）；§12 补 vitest 的
  `resolve.dedupe`（顺带一句收尾用 `dispose()`，ipdrun-03）。
- 测试：`tests/app-scripts.mjs`（offline 层）的「build.sh: without its own workspace an app installs only harnix/」「build.sh: with its own workspace
  the app's packages are installed before the shell is built」：用记录调用的假 `pnpm` / `cargo` 跑真 `build.sh`，有没有 `pnpm-workspace.yaml` 各一遍；
  改动前第二条失败（已验证）。
- 偏离：验收写的是在 `tests/app-template.mjs` 里用模板建一个带第三方依赖的 space 的应用、真跑 `build.sh`——那要联网装包，template 层是离线构建；
  改用上面的离线行为测试。
