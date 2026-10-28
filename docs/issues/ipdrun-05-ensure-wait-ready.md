# ipdrun-05：spawn 实例 `ensure` 返回时还在 starting，紧接着的会话调用失败

类型：新能力｜优先级：高

## 现象

模块调 `ctx.instances().ensure(template, name, params)`（`crates/module/src/sessions.rs:251-263`）建一台 spawn 实例，
立刻返回实例视图，`status.state == "starting"`：`DaemonLink` 在后台起子进程、等它监听、握手（`crates/instance/src/daemon_link.rs:229-245`），
通常要 0.1–0.4 s。同一个任务里接着 `ctx.sessions().create(SessionSpec { instance: name, … })`，中枢转给链接，链接答
`INSTANCE_UNAVAILABLE "the instance is starting"`（`crates/instance/src/link.rs:109-117`）。

进程内实例没有这个问题（`build` 返回时引擎已建好）。spawn 实例被看护重启期间（`Degraded`）同理。

每个用 spawn 模板的模块都得自己写「轮询 `harnix.instances.list` 等 `ready`、`down` 立刻报错、带超时」这一段。

## 建议

只加不改：**`ensure` / `create` 可以等到 ready。** 方法参数加可选的 `wait`：
```jsonc
// harnix.instances.ensure / .create
{ "template": "...", "name": "...", "params": {...}, "wait": "ready", "timeout_ms": 60000 }
```
`wait: "ready"` 时，返回前等这台的状态变成 `ready`；变成 `down` 立即以 `INSTANCE_UNAVAILABLE` 报错（带 `down` 的理由）；
超时同样报错。不带 `wait` 时行为与现在一样。模块侧加对应的便捷方法：
```rust
impl Instances<'_> {
    pub async fn ensure_ready(&self, template: &str, name: &str, params: BTreeMap<String, String>, timeout: Duration)
        -> Result<Value, RpcError>;
    pub async fn wait_ready(&self, instance: &str, timeout: Duration) -> Result<Value, RpcError>;
}
```
等待用链接自己的状态变化通知（不必轮询）。

会话类调用（`session.create`、`harnix.send` 等）遇到 `Starting` 的实例照旧立即答 unavailable，不在中枢里等：那会改缺省行为，
依赖「马上失败」做回退的调用方会变慢。要等的调用方显式用 `wait: "ready"`。

并在 SDK §6.2 写明：spawn 实例 `ensure` 之后不是马上可用，用 `wait: "ready"` 或 `ensure_ready`。

## 兼容性

- 纯新增：不带 `wait` 的调用一字不变；`Instances::ensure` / `create` 不变；会话类调用遇到 starting 的实例照旧马上失败。
- 协议版本不必升：新参数可选，旧服务端不认识时报 `invalid_params` 的话，模块能看出来。
- Helm、hello-app 不用 spawn 模板，不受影响。

## 验收

- `tests/live/api/instances-daemon.spec.ts`：`ensure { wait: "ready" }` 返回时 `status.state == "ready"`，紧接着 `session.create` 成功；
  不带 `wait` 的 `ensure` 仍可能返回 `starting`（不断言，只断言不报错）。
- 一个起不来的 spawn 模板（`program` 指向不存在的路径或立即退出的脚本）：`wait: "ready"` 在 `down` 后马上报 `-31004`，不等到超时。
- `timeout_ms: 1` 对一台慢启动的模板：报超时错误。
- 模块侧：`crates/testapp` 用 `ensure_ready` 建 spawn 实例后立刻建会话，测试通过。

## 依赖

无。与 `ipdrun-04`（并发 ensure）独立；两条都修了，模块就不需要自己的「串行 + 轮询」那一层。

## 来源

IPDrun 改用 spawn 模板后，API 测试里每台实例建好后的第一次 `session.create` 都失败；现在在宿主里 ensure 后轮询 `instances.list`
等到 ready（`crates/agent/src/host.rs` 的 `wait_ready`）。
