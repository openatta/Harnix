# ipdrun-04：两个并发的 `instances.ensure` 同名时，第二个失败

类型：缺陷｜优先级：高

## 现象

`harnix.instances.ensure` 的文档说「已经按这个模板建过同名实例就返回它」（`crates/hub/src/instances.rs:212-215`）。
实现分两步，中间没有锁：

1. `create_instance` 在**锁外**查「已有就返回」（`instances.rs:249-253`）；
2. 没有就调 `build()`，`build()` 在**锁内**再查一次，撞名就报 `invalid_params("there is already an instance `{name}`")`
   （`instances.rs:276-289`）。

两个任务同时 `ensure` 同一个名字：都在第 1 步看到「没有」，一个建成，另一个在第 2 步收到「already an instance」。
`build_owned`（`:258-265`，space 的默认实例）是同样的结构。

对调用方来说，`ensure` 的意义就是「幂等、不用自己协调」；现在模块只能在外面再包一把互斥锁，把所有 `ensure` 串行。

## 建议

不改接口，修实现：`build()` 在锁内撞名时，若调用方是 `ensure`（或 `build_owned`）、且已有的那台是同一个模板建的，
返回已有的那台的视图，而不是报错。最直接的做法是把 `ensure: bool` 传进 `build()`，锁内的那次检查按它分支：

```rust
if let Some(existing) = self.slot(&name) {
    if ensure && existing.template.as_deref() == Some(template_id.as_str()) {
        return Ok(self.instance_view(&existing));
    }
    return Err(RpcError::invalid_params(format!("there is already an instance `{name}`")));
}
```

锁外那次检查可以留着当快路径。`create`（非 ensure）撞名照旧报错。

## 兼容性

- 签名、参数、返回形状不变。
- 行为变化只在「并发 ensure 同名」：以前其中一个报错，现在都拿到同一台实例。没有调用方能依赖那次报错。
- 同名、不同模板的 `ensure` 照旧报错（与串行时一致）。

## 验收

- `tests/live/api/instances.spec.ts`：同一个模块并发 `ensure` 同一个名字 8 次（`Promise.all`），全部成功、返回同一个 `id`，
  `instances.list` 里只有一台。进程内模板与 spawn 模板各一遍（spawn 的放 `instances-daemon.spec.ts`）。
- 并发 `create` 同名 2 次：恰好一个成功、一个 `invalid_params`（回归）。

## 依赖

无。和 `ipdrun-05`（等到 ready）互相独立，可以一起发。

## 来源

IPDrun 的模块为同一个工作单元并发建两台实例时撞上这个报错，现在在自己的宿主里把 `ensure` 串行（`crates/agent/src/host.rs` 的 `ensuring` 互斥）。

## 实现记录

- `crates/hub/src/instances.rs`：`build()` 多一个 `ensure` 参数，锁内撞名时，`ensure`（含 `build_owned`）且同模板就返回已有的那台；
  另外把 `drop(registry)` 挪到 `add_instance` 之后——原来两者之间还有一个窗口：第二个调用拿到锁时第一台还没进实例表，会再建一台、
  顶掉第一台。`create` 撞名照旧 `invalid_params`。
- testapp 的 `fleet` 加 `app.fleet.ensure_many {template, name, count}`（一个模块并发 `ensure`）。
- 测试：`tests/live/api/instances-ready.spec.ts` 的「eight ensures of one name at once, from a module, are one in-process instance」
  「…from a device, are one spawned instance」「two creates of one name at once: one is built, the other refused」。
- 偏离：spawn 的那一遍放在新文件 `instances-ready.spec.ts`（与 05 共用一个后端），没有放进 `instances-daemon.spec.ts`。
