# ipdrun-02：SIGTERM 不走模块的 `stop`

类型：缺陷｜优先级：高

## 现象

`crates/app/src/run.rs:572-576` 只在三件事之一发生时进入收尾：服务出错、`tokio::signal::ctrl_c()`（SIGINT）、
`--exit-with-stdin` 下 stdin 关闭。收尾依次调各模块的 `stop()`、`hub.shutdown()`、删本进程的上传目录（`:578-591`）。

SIGTERM 不在里面。`kill <pid>`、launchd / systemd 停服务、进程管理器、测试工具发 SIGTERM 时，进程按默认处置直接退出：

- 模块的 `stop()` 不跑：模块写的运行记录（pid 文件之类）留着，模块起的子进程成了孤儿——tokio 运行时不 drop，
  持着 `Child` 的 future 不析构，`kill_on_drop` 也就不生效；
- `hub.shutdown()` 不跑，进程内实例的在跑的轮没有收尾；
- `<状态目录>/uploads/<pid>/` 留着。

## 建议

`run.rs` 的 `select!` 加一支 SIGTERM（unix 下 `tokio::signal::unix::signal(SignalKind::terminate())`），走与 Ctrl-C 同一条收尾：

```rust
#[cfg(unix)]
let terminate = async {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    term.recv().await;
    Ok::<_, std::io::Error>(())
};
tokio::select! {
    result = server => { result??; }
    _ = tokio::signal::ctrl_c() => println!("shutting down…"),
    _ = terminate => println!("terminated; shutting down…"),
    _ = orphaned => println!("stdin closed; the parent is gone — shutting down…"),
}
```

另外两点：

- 收尾期间再收到一次 SIGTERM 或 SIGINT，立即退出（不等模块）：给卡住的 `stop()` 一个出口，也合进程管理器「再发一次就是强停」的习惯。
- 收尾整体有一个上限（例如 10 s），超了记一条日志后退出；模块文档（SDK §5.2 `stop`）写明这个上限。

## 兼容性

- 行为变化只在 SIGTERM：以前直接退出，现在先跑 `stop()` 再退出。这是要修的缺陷。
- 依赖「SIGTERM 立刻死」的调用方：退出会晚一点（模块 `stop` 的耗时）；第二次信号或上限保证不会无限等。
  testkit 的 `halt()` 本来就等 `exit` 事件，不受影响；`stop()` 见 `ipdrun-03`。
- 模块接口不变。自己在 `start` 里装了 SIGTERM 处理的模块：两边都会收到信号，模块应当去掉自己的那份（在 SDK 里提一句）。
- Helm：它的模块起 Python 子进程，SIGTERM 下同样受益。

## 验收

- `tests/live/api/modules.spec.ts`（用 `crates/testapp` 的模块）：模块在 `stop()` 里写一个标记文件；对进程发 SIGTERM，
  进程退出后标记文件存在。SIGINT 同样（回归）。
- 模块 `stop()` 故意卡住：第二次 SIGTERM 后 1 s 内进程退出。
- `uploads/<pid>/` 在 SIGTERM 后不存在。

## 依赖

无。

## 来源

IPDrun 的服务被 `kill` 后，模块的运行记录残留、执行器起的样例服务和测试进程成了孤儿（API 测试跑完曾留下约 90 个）；
IPDrun 现在在模块 `start` 里自己接 SIGTERM、清进程组后立即退出（`crates/module/src/lib.rs` 的 `sigterm_kills_exec_groups`）。
