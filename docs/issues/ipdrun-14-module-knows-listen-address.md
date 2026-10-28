# ipdrun-14：模块不知道进程监听在哪个地址

类型：新能力｜优先级：低

## 需求

模块常常要把「怎么连到这个进程」告诉别人：写给自己的命令行工具读的运行记录、给外部 Agent 的 MCP 配置、日志里的一行链接、
给桌面壳或通知用的 URL。`ModuleContext`（`crates/module/src/context.rs:75-180`）有 id、配置、数据目录、凭据、事件、调用，
没有监听地址。

而且模块的 `start` 在监听**之前**跑（`crates/app/src/run.rs:370-390` 起模块，`:483-493` 才定地址并开始服务），
所以模块即使自己去读 profile 与命令行（`--host`、`--port`），也只能复制 Harnix 的解析规则，`--port 0` 时还拿不到真实端口。

## 建议

只加不改：

```rust
pub struct Listen {
    /// Every address this process accepts on (both loopback families when bound to loopback).
    pub addrs: Vec<std::net::SocketAddr>,
    /// The one to give a person or a tool: `http(s)://<first addr>/`.
    pub url: String,
}

impl ModuleContext {
    /// Where this process listens, once it does; `None` while modules are still starting.
    pub fn listen(&self) -> Option<Listen>;
    /// The same, waiting until the listener is bound.
    pub async fn listening(&self) -> Listen;
}
```

`run.rs` 绑定成功后把真实地址（`--port 0` 时取 `local_addr()`）写进一个共享的 `OnceLock`，所有模块的上下文都读它。模块在 `start` 里
`tokio::spawn` 一个任务 `listening().await` 再写自己的运行记录即可。

## 兼容性

- 纯新增：不调这两个方法的模块不受影响；`Module` trait、`MODULE_API_VERSION` 不变。
- 地址里不含令牌；令牌的去处不变。
- Helm、hello-app 不受影响。

## 验收

- `crates/testapp` 的模块在 `start` 里 spawn 一个任务，`listening().await` 后把 `url` 写进自己的数据目录；`tests/live/api/modules.spec.ts`
  读这个文件，它等于 testkit 拿到的 `Backend.url`。
- 用 `--port 0` 起（若支持）：文件里是真实端口，不是 0。
- `start` 里同步调 `listen()`：返回 `None`（文档化的行为）。

## 依赖

无。与 `ipdrun-09` 的锁文件里写监听地址是同一个数据，可以一起做。

## 来源

IPDrun 的模块要把服务地址写进自己的运行记录给命令行用，现在自己解析 `--host` / `--port` 与 profile（`crates/module/src/run.rs`、`app/src/cli.rs`）。

## 实现记录

- `harnix::Listen { addrs, url }`；`ModuleContext::listen() -> Option<Listen>`、`ModuleContext::listening().await -> Listen`。实现上是 `Kernel` trait
  多一个带缺省实现的 `fn listen(&self) -> Option<watch::Receiver<Option<Listen>>>`（缺省 `None`，旧的 `Kernel` 实现不用改），应用的
  `ProcessKernel` 持一个 `watch`。
- `harnix_transport::serve_with(addrs, router, tls, bound)`：全部绑好后把实际地址交给 `bound`；`serve` 不变（调它，`bound` 为空操作）。
  `--port 0` 时第二个地址（IPv6 回环）沿用第一个拿到的端口，两族同一个端口。
- 偏离：建议里写的是共享的 `OnceLock`；用 `watch` 是为了 `listening()` 能等而不轮询。
- 测试：Rust `serve_tests::port_zero_is_reported_as_the_port_bound`（`--port 0` 报的是真实端口，两族一致）；`tests/live/api/modules.spec.ts` 的
  「a module learns where the process listens, once it does」（testapp 的 `lifecycle` 在 `start` 里记下 `listen()` 为 `None`，再 spawn 任务
  `listening().await` 写 `listen.json`；`url` 等于 testkit 的 `Backend.url`）。testkit 起进程时自己选端口，没有用 `--port 0` 起整个进程的 live 用例。
