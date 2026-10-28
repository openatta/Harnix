# ipdrun-03：testkit 的 `stop()` 不等进程退出就删目录

类型：缺陷｜优先级：中

## 现象

`@harnix/testkit` 的 `Backend.stop()`（`ui/packages/testkit/src/launcher.ts:227-234`）：

```ts
stop() {
  try { child.kill(); } catch { /* already gone */ }
  rmSync(scratch, { recursive: true, force: true });
},
```

`child.kill()` 发 SIGTERM 后立刻 `rmSync`。进程还在收尾（模块 `stop()`、引擎落盘、spawn 实例的 daemon 退出）时仍在往
`scratch` 下写文件，`rmSync` 于是撞上 `ENOTEMPTY` 抛错，或者删完以后又被写回几个文件、目录残留。`ipdrun-02` 修好以后
SIGTERM 会走完整收尾，这个竞争只会更常见。

同文件的 `halt()`（`:208-217`）已经是「发信号、等 `exit`」的写法，`stop()` 没有复用它。

## 建议

只加不改：

```ts
export interface Backend {
  // …原有成员不变
  /** End the process, wait until it has exited, then remove everything it owned. */
  dispose(): Promise<void>;
}
```

- `dispose()`：`await halt()`（等 `exit`，带上限，超了 SIGKILL 再等），然后 `rmSync(scratch)`；删的时候遇到 `ENOTEMPTY` / `EBUSY`
  短暂重试几次。
- `fixtures.ts:67` 与 `suite.ts:65` 的收尾改用 `await dispose()`。
- `stop()` 保持同步签名不变，文档注明「不等进程退出；收尾里用 `dispose()`」。

## 兼容性

- `stop()` 的签名与行为不变，现有测试照常编译、运行。
- 用 testkit 夹具的应用（Helm 的 `tests/`、hello-app）自动受益：夹具换成 `dispose()` 以后，收尾不再与进程抢目录；
  测试代码自己调 `stop()` 的地方不受影响。
- 收尾多等一个进程退出的时间（通常几十到几百毫秒）。

## 验收

- `tests/live/api/` 加一条：起一个模块在 `stop()` 里慢慢写文件的 testapp（例如 500 ms 内写 20 个文件），调 `dispose()`，
  返回后 `scratch` 不存在、进程不在。
- 同样的场景调 `stop()`：行为与现在一致（只断言不抛到测试外，不断言目录已删）。

## 依赖

无；与 `ipdrun-02` 一起落地效果最好。

## 来源

IPDrun 的 API 测试收尾时删目录撞上 `ENOTEMPTY`，现在在自己的测试工具里加了重试（`qa/kit/ipdrun.ts`）。

## 实现记录

- `ui/packages/testkit/src/launcher.ts`：`Backend.dispose()`：发 SIGTERM 等 `exit`，15 s（`DISPOSE_GRACE_MS`，过了后端自己的 10 s 收尾时限）
  后 SIGKILL 再等；然后删 `scratch`，遇 `ENOTEMPTY` / `EBUSY` / `EPERM` 每 100 ms 重试，最多 20 次。`stop()` 签名与行为不变，文档注明不等退出。
- `fixtures.ts`、`suite.ts` 的收尾改为 `await dispose()`。
- 测试：`tests/live/api/shutdown.spec.ts` 的「dispose waits for the process to finish before removing its directory」（`lifecycle` 在 `stop` 里
  500 ms 写 20 个文件；返回后进程不在、目录不在）、「stop still returns at once, without waiting for the process」。
- 偏离：testkit 在 `ui/` 下，本条由后端批次（F3a）一并做了，外壳批次不必再做。
