# ipdrun-07：spawn 实例停下以后，它的会话起的后台进程还活着

类型：缺陷｜优先级：高

## 现象

spawn 实例的 daemon 由 `DaemonLink` 起（`crates/instance/src/daemon_link.rs:229-245`：stdin 管道、`kill_on_drop`），停的时候
`stop_child` 关 stdin、等 5 s、不退就 `kill` 这一个 pid（`:397-411`）。

daemon 里会话跑的命令各自成一个进程组（Agix 的 `LocalProcess`），命令结束后留在后台的进程（`nohup … &`）既不在 daemon 的
进程组里，也不会被 daemon 的退出带走：

- `instances.stop` 之后 daemon 3 ms 就退了，后台进程还在，父进程变成 1；
- daemon 被 `kill`（5 s 超时那一支）或崩溃时，连它正在跑的命令的组都没人收；
- 看护重启（`Degraded` → 重新 spawn）时，旧 daemon 留下的进程与新 daemon 并存。

Harnix 能拿到的只有 daemon 的 pid，模块无从知道这台实例起过哪些进程。

## 建议

模板可以声明「停的时候收干净」，缺省关：

```rust
impl InstanceTemplate {
    /// A spawned daemon gets a session of its own; stopping or restarting it ends every process
    /// still in that session.
    pub fn reap_on_stop(mut self) -> Self;
}
```

声明了的模板：

- spawn 时让 daemon 自成一个会话（`pre_exec` 里 `setsid()`）。Agix 的命令虽然各自 `setpgid`，仍留在这个会话里。
- `stop_child`、看护发现 daemon 退出、`instances.stop` 之后：找出会话号等于 daemon pid 的所有进程，TERM，等一个宽限，KILL。
  列进程是平台相关的（Linux 读 `/proc/*/stat` 的 session 字段；macOS 用 `proc_listallpids` + `getsid`），放在 `harnix-instance`
  一处，测试覆盖两个平台。
- daemon 起的时候顺带打开 Agix 的会话级回收（Agix `ipdrun-01` 的 `sandbox.reap_process_groups`），正常退出时由引擎自己收，
  Harnix 这一步只兜「daemon 没来得及收」的情况。
- 实例视图（`instance_view`，`crates/hub/src/instances.rs:496-511`）可选地多一个 `reap: true`，方便诊断页显示。

## 兼容性

- 不声明 `reap_on_stop` 的模板：行为完全不变，故意让后台服务活过实例的应用不受影响。
- 声明了的：daemon 换成自己的会话。它不再收到发给 Harnix 进程组的终端信号（Ctrl-C）——本来就由 stdin 关闭与 `stop_child` 管它的退出，
  所以没有功能损失；文档写明。
- 进程内实例不适用（会话就是 Harnix 自己的）；进程内的回收靠 Agix `ipdrun-01`。
- Helm、hello-app 不用 spawn 模板，不受影响。

## 验收

- `tests/live/api/instances-daemon.spec.ts`：一个声明了 `reap_on_stop` 的 spawn 模板（testapp），会话里 Bash 跑
  `sleep 300 >/dev/null 2>&1 & echo $!`，拿到 pid；`instances.stop` 后 3 s 内这个 pid 不在了。
- 同样的用例对 daemon 发 `kill -9`：看护发现退出后 3 s 内后台进程不在。
- 不声明的模板：`stop` 之后后台进程还在（缺省行为不变的回归），测试自己收尾。
- Linux 与 macOS 都跑（CI 能跑哪个写明哪个）。

## 依赖

正常退出时的那一半依赖 Agix `ipdrun-01`；`kill -9` / 崩溃那一半不依赖，可以先做。

## 来源

IPDrun 的实例里模型用 Bash 起了 `nohup sleep … &`，`instances.stop` 之后进程一直在；IPDrun 现在按「父进程是 1、工作目录在实例的
工作区下」去找孤儿进程组再杀（`crates/agent/src/reap.rs`），是启发式，`cd` 出去的找不到。
