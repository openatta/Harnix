# ipdrun-43：桌面模板的壳不能当自己子进程的客户端

类型：新能力｜优先级：低

## 需求

壳替自己做的事——系统通知、Dock 角标、原生对话框选的项目——要以客户端身份调子进程的方法、收主机事件（页面一侧不加通道）。模板
（`templates/desktop/src-tauri/src/local.rs`）只会 `POST /auth/link`（手写的一次 HTTP），应用要再写一份握手客户端，握手的版本号壳拿不到
（壳不链 Harnix），只能在 `build.rs` 里从 Harnix 源码读常量；慢方法、chunked 应答也要自己处理。

## 建议

模板依赖 ipdrun-42 的 `harnix-client`（按路径引用；应用拷过去时改成自己的 `harnix/crates/client`）：

- `mint` 改用 `harnix_client::launch_link`，行为不变（令牌不对、连不上、别的错误三种说法照旧）；
- `Started::client()`：读进程令牌、握手，交回 `harnix_client::Client`——应用的壳拿它 `call` / `call_within`、`events(Face::Host, …)`；
- README「本地模式」写壳当客户端的做法。

## 兼容性

模板多一个依赖（`harnix-contract` 与 `serde_json` 之外不带别的）；壳的行为不变，仍不经 Tauri IPC（`check-no-ipc.mjs` 通过）。

## 验收

模板 `cargo check` 无警告；`check-no-ipc.mjs`、`check-rename.mjs` 通过；`--smoke`（有应用二进制的机器上）照旧走启动链接。

## 依赖

ipdrun-42。

## 来源

IPDrun 的桌面壳为通知、Dock 角标、原生打开项目手写了第二份握手客户端（`desktop/src-tauri/src/client.rs`），`build.rs` 从 Harnix 源码读版本常量，
`local.rs` 自己拆 chunked（HARNIX-WORKAROUND C1-#30，N12）。
