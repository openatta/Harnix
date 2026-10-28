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

## 实现记录

（F3c，与 ipdrun-10、ipdrun-13 同一提交，提交号见 `git log --grep ipdrun-07`；Agix 钉在 `Agix-v0.2.9-2026102701`，`bbdd8e2`。）

- `InstanceTemplate::reap_on_stop()`（字段 `reap_on_stop`，缺省 false；profile 的 `[[instance_templates]]` 写 `reap_on_stop = true`）。
  `Factory::build_template` 读它：两种载体都在实例数据根的全局层 `settings.json` 里打开 Agix 的 `sandbox.reap_process_groups`
  （写文件而不是只改内存：引擎按项目从磁盘读各层设置，`config.reload` 也重读；文件里其他键不动）。spawn 时 `DaemonLink::start_with(spec, SpawnOptions::default().reap_on_stop())`：
  `pre_exec` 里 `setsid()`；`stop_child`（`instances.stop`、关进程）在 daemon 退出 / 被 kill 之后、`ensure_child` 发现 daemon 已退出要重启之前，
  对会话号等于旧 daemon pid 的进程 TERM，等 `REAP_GRACE`（1.5 s），KILL 剩下的。列进程在 `crates/instance/src/reap.rs` 一处：Linux 读
  `/proc/<pid>/stat` 的 session 字段（跳过僵尸），macOS 用 `proc_listallpids` + `getsid`；永远不收本进程自己的会话。
- 实例视图：`EngineLink::reaps_on_stop()`（缺省 false，只加）为真时多一项 `reap: true`。
- 测试：Rust `reap::tests::a_session_whose_leader_was_killed_is_reaped`、`reap::tests::this_process_never_reaps_its_own_session`、
  `reap::tests::the_engine_s_reaping_is_turned_on_in_the_global_tier_and_nothing_else_moves`；`tests/live/api/instances-daemon.spec.ts` 的
  「reap_on_stop: stopping the instance ends what its sessions left running」「reap_on_stop: a daemon killed outright has its session's processes
  ended when it is restarted」「without reap_on_stop, what a session left running outlives the instance, as before」。新录像 `leaves-a-process`
  （由 `calls-a-tool` 手改出：只换了工具入参和最后的文字）。

偏离：

- 验收里的 `sleep 300 … &` 换成 `tail -f /dev/null … &`：Agix 拒绝 Bash 里 2 s 以上的 `sleep`（`use the Monitor tool`）。
- 模板放在测试的 profile（`[[instance_templates]]` 的 `reaper`、`keeper`）而不是 testapp 的模块里：设备只能按 `ui` 模板建，profile 模板正好；
  为此 profile 也认 `reap_on_stop`。
- 只在 macOS 上跑过（本机）；Linux 那一支有代码、未在 CI 上跑。
- 进程内实例的视图不带 `reap`（`InProcessLink` 没有这一项；它的回收只是引擎那一半）。
- 实例被 kill -9 之后的回收发生在看护**重启**它之前；看护若因连续快速退出放弃（`down`），最后一次退出后的会话不再收。
