# ipdrun-06：spawn 模板的缺省程序不能在运行时覆盖

类型：新能力｜优先级：低

## 需求

`TemplateCarrier::Spawn { program: None, .. }` 的意思是「本二进制的 `daemon` 子命令」（`crates/instance/src/templates.rs:80-86`）。
`Factory::new` 把它定成 `std::env::current_exe()`（`crates/instance/src/factory.rs:51`），`program` 为 `None` 时用它（`:93`）。

在 Rust 集成测试里进程内起 Harnix（`App::builder()…run()` 跑在 libtest 的二进制里）时，`current_exe()` 是测试二进制，
它没有 `daemon` 子命令，spawn 模板就起不来。应用要么给每个模板写死 `program`（失去「同一个二进制、协议版本天然一致」的好处），
要么自己加一个配置项再在模板里读，各写各的。

## 建议

环境变量 `HARNIX_DAEMON_PROGRAM`：设了就替换 `Factory` 的缺省程序（即 `program: None` 时用的那个），模板自己写了 `program`
的不受影响。

```rust
// factory.rs, Factory::new
let program = match std::env::var_os("HARNIX_DAEMON_PROGRAM") {
    Some(p) => PathBuf::from(p),
    None => std::env::current_exe()?,
};
```

启动时打一条日志说用的是哪个程序；SDK §12（测试）写一句「在测试二进制里跑 spawn 模板，把 `HARNIX_DAEMON_PROGRAM` 指向应用的二进制」。

## 兼容性

- 不设这个变量：与现在完全一样。
- 模板显式写了 `program` 的：不受影响。
- 安全：这是启动进程的人设的环境变量，与 `program` 写在模板里同一信任级别；界面与模块都设不了它。

## 验收

- `crates/instance` 的单元测试：设了变量时 `Factory::new().program` 是它，不设时是 `current_exe()`。
- `crates/harnix/tests/main.rs` 一类的进程内集成测试：在测试二进制里设 `HARNIX_DAEMON_PROGRAM=<harnix 二进制>`，建一台 spawn 实例并跑通一轮。

## 依赖

无。

## 来源

IPDrun 的 API 测试在测试二进制里起服务，spawn 模板找不到 `daemon` 子命令；IPDrun 加了自己的 `[modules.ipdrun] daemon = <路径>`
配置项（`crates/module/src/config.rs`）。
