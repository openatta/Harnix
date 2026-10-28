# ipdrun-08：新旧两个 daemon 会同时写同一个实例数据根

类型：缺陷｜优先级：中

## 现象

spawn 的 daemon 靠 stdin 关闭得知父进程没了（`crates/instance/src/serve.rs:42-50`），然后**优雅收尾**：停止监听、
`engine.shutdown()` 等在跑的轮结束、落盘（`serve.rs:58-60`；`crates/instance/src/engine.rs:96`）。实测要 2 s 上下。

Harnix 进程被 SIGKILL（或崩溃）后马上在同一个状态目录重启（进程管理器、桌面壳、测试工具都会这么做），新进程 0.5 s 就绪，
按账本在**同一个数据根**上起了新的 daemon（`crates/hub/src/instances.rs:163` 的 `restore`）。于是两个引擎重叠一两秒，
同时写同一个会话的转录、sidecar（名字）、设置文件。数据根上没有任何互斥（全仓库没有文件锁）。

## 建议

daemon 对自己的数据根持一把独占锁，活多久持多久：

- `serve_daemon` 一开始在 `<data_root>/.daemon.lock` 上 `flock(LOCK_EX | LOCK_NB)`；拿不到就按间隔重试，最多等一个上限
  （例如 10 s，可由 `--lock-wait <secs>` 改），等到了再建引擎；超时以非零退出，`DaemonLink` 把实例记成 `down`，理由写
  「data root is held by pid N」。锁文件里写持有者的 pid，方便理由和诊断。
- 进程内实例同样在建引擎前拿这把锁（同一个数据根不该被两个引擎同时打开，不论载体）。
- 可选：`DaemonArgs` 加 `--exit-grace <secs>`，stdin 关闭后在跑的轮最多等这么久就退出（缺省不限，即现在的行为）。
  应用想让「父进程没了」更快让出数据根时可以设它。

## 兼容性

- 正常情况下（只有一个引擎用这个数据根）没有可见变化，只多一个锁文件。
- 重叠的情况下：新 daemon 晚一两秒就绪，而不是与旧的并写。等不到时报 `down`，比静默并写好。
- 锁是建议性锁，只约束 Harnix 自己起的引擎；不影响别的工具读数据根。
- `--exit-grace` 不设时行为不变。

## 验收

- `crates/instance` 测试：在一个数据根上起两个 `serve_daemon`，第二个在第一个退出前不建引擎；第一个退出后第二个在 1 s 内就绪。
- 锁被占满 `--lock-wait` 之后，第二个以非零退出，`DaemonLink` 的状态是 `down` 且理由含持有者 pid。
- `tests/live/api/instances-daemon.spec.ts`：对 Harnix 进程 SIGKILL 后立即在同一目录重启，新 daemon 就绪前旧 daemon 已退出
  （查 pid）；会话转录完整、没有交错的行。

## 依赖

无。

## 来源

IPDrun 的服务被杀后马上重启，同一条会话在新旧两个 daemon 里重叠约 1.8 s、转录与名字被两边同时写；IPDrun 现在在启动时把
`--data-root` 落在自己实例目录下、父进程是 1 的旧 daemon 杀掉（`crates/agent/src/reap.rs` 的 `kill_stale_daemons`）。
