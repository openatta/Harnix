# ipdrun-09：同一个状态目录可以同时起两个进程

类型：缺陷｜优先级：中

## 现象

一个应用的状态目录（账本，`~/.<product_id>/`）里放着令牌、设备、实例账本（`instances.json`）、模块数据。启动时没有任何互斥：

- `crates/app/src/run.rs:131` 读实例账本，`:406-409` 生成新令牌写进 `<状态目录>/token`；
- 端口不同（或 `--port 0`、桌面壳每次挑空闲端口）时，第二个进程照常起来：它覆盖第一个的令牌文件（之后按令牌文件连第一个的
  客户端被拒）、按同一份账本在**同一个实例数据根**上再建一遍实例、两边各写各的账本与模块数据。

只有端口相同时第二个才会因端口被占而起不来——那是碰巧，不是保证。

## 建议

进程对状态目录持一把独占锁，活多久持多久：

- 读账本之前在 `<状态目录>/harnix.lock` 上 `flock(LOCK_EX | LOCK_NB)`；拿不到就以非零退出，提示
  「`<状态目录>` is in use by pid N (http://127.0.0.1:P/)」。
- 锁文件里写持有者的 pid 与监听地址（监听之后补写）。这样「是不是已经有一个在跑、在哪个地址」有了一个可靠的读法，
  桌面壳和命令行工具不必再去猜（与 `ipdrun-14` 的监听地址互补）。
- `harnix daemon` 子命令不拿这把锁（它用实例数据根的锁，见 `ipdrun-08`）。

## 兼容性

- 只有一个进程用这个状态目录时：没有可见变化，多一个锁文件。
- 两个进程用同一个状态目录：以前第二个「起来了但互相踩」，现在第二个明确拒绝。这是要修的缺陷；需要两个实例的应用本来就该用
  两个状态目录（`HARNIX_STATE_DIR` 或不同的 `product_id`）。
- testkit 每个后端各有自己的 `HOME`，不受影响；`restart()` 先 `halt()` 等旧进程退出再起，锁已释放。
- Helm、hello-app：不改代码。

## 验收

- `tests/live/api/` 加一条：同一个 `HOME`、不同端口起第二个进程，它在 5 s 内以非零退出，输出里有第一个的 pid；第一个照常服务，
  令牌文件没被改。
- 第一个被 SIGKILL 后，第二个能立刻起来（`flock` 随进程消失释放）。
- testkit 的 `restart()` 用例照常通过（回归）。

## 依赖

无。

## 来源

IPDrun 迁到 Harnix 后失去了自己原来的状态目录锁，它的「同一状态目录起第二个实例应当被拒」的 API 用例现在只是因为端口相同才通过
（`app/tests/api/project.rs` 的 `second_instance_with_same_state_dir_is_refused`）。

## 实现记录

- `crates/app/src/state_lock.rs`：`<状态目录>/harnix.lock`，`flock(LOCK_EX | LOCK_NB)`（unix；非 unix 不加锁，同以前），在 `run.rs` 解析完目录、
  读任何东西之前拿；拿不到就返回错误「`<目录>` is in use by pid N (url)」，进程以非零退出。锁文件内容 `{"pid": N, "url": null}`，监听后补写
  `url`（与 ipdrun-14 同一份地址）。`harnix daemon` 子命令在这之前就分出去了，不拿这把锁。
- 测试：Rust `state_lock::tests::a_second_holder_is_refused_with_the_first_ones_pid_and_url`；`tests/live/api/state-dir.spec.ts` 的
  「a second process on the same state directory is refused, naming the first」（同一 HOME、另一端口，5 s 内非零退出、输出有第一个的 pid 与 URL，
  令牌文件不变、第一个照常答）、「after the holder is killed outright, the next one starts at once」（SIGKILL 后 `restart()` 立即起来）；
  testkit `restart()` 的回归在 `migrations.spec.ts` 里照常通过。
