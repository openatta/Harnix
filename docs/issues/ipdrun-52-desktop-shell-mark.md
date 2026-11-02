# ipdrun-52：页面不知道自己在桌面壳里、是不是本地模式（`window.__HARNIX_SHELL__`）

类型：新能力｜优先级：低

## 现象或需求

有的操作只在桌面壳的本地模式下才有意义：「用系统程序打开」一个项目文件（文件在这台机器上、壳能起系统程序）。页面现在只能从
Tauri 放进每个页面的内部对象（`__TAURI_INTERNALS__`，`ui/packages/shell/src/main.tsx` 的 `chrome()`）猜"在壳里"，猜不出是本地还是远程模式；
按设计又不能给页面开命令（`check-no-ipc.mjs`）。IPDrun 只能留一个永远不出现的钩子（`window.__IPDRUN_DESKTOP__.openPath`）。

## 建议

模板的壳在建窗口时给页面一段初始化脚本（`WebviewWindowBuilder::initialization_script`），只定义一个冻结的只读标记：

```js
window.__HARNIX_SHELL__ = { kind: "desktop", mode: "local" | "remote", platform: "macos" | "windows" | "linux" | … }
```

- 单向：没有函数，页面够不着壳；页面要壳做的事照旧请求自己的服务，服务经壳以客户端身份连着的链接转给壳（ipdrun-43 的做法）。
- `mode` 取 `desktop.toml` 的解析结果：远程模式 `remote`，其余（含配置读错、落到错误页）`local`。
- README「业务流量的规则」写明。

## 兼容性

只加；`check-no-ipc.mjs` 不变（初始化脚本不在禁用表里，也不是通道）。拷了模板的应用不改就没有这个标记，页面照旧。

## 验收

模板 `cargo check` 无警告；`check-no-ipc.mjs` 通过；IPDrun 的桌面壳照做，项目 space 的「用系统程序打开」在本地模式出现。

## 依赖

ipdrun-43（壳当自己子进程的客户端）。

## 来源

IPDrun R11b 偏离③（stage-R11.md §3）；R12 收口（stage-R12.md §1.1）。

## 落地

落地提交：`18aec65`（main，2026-11-02，未推送；版本待下次发布）。
模板 `src-tauri/src/main.rs` 的 `shell_mark`（`initialization_script`）；README「业务流量的规则」一段。模板 `cargo check` 无警告、`check-no-ipc.mjs` 通过；
`--smoke` 没跑（本机没有打好的应用二进制）。
