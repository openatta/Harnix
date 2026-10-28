# ipdrun-10：spawn 模板带不了原生贡献

类型：新能力｜优先级：中

## 需求

进程内模板可以带原生贡献：`TemplateCarrier::InProcess { engine, contrib }`（`crates/instance/src/templates.rs:73-79`），
模块借它给实例加场景、原生工具、`customize_session`。spawn 模板没有这一项（`templates.rs:80-86`）：daemon 子命令
（`crates/app/src/run.rs:51` → `crates/instance/src/serve.rs:26-32`）建引擎时用的是 `EngineContrib::default()`。

于是同一个模块想把一类实例从进程内换成 spawn（为了隔离、为了单独重启），就丢掉它的原生场景和会话定制，只能退回设置层的
command hook、`prompt_append` 之类的配置手段，两种载体各写一套。

daemon 子命令跑的就是应用自己的二进制，模块的代码都在；缺的只是「这台 daemon 是哪个模板的」这一条信息。

## 建议

1. 模板可以声明 spawn 时也要的原生贡献（只加不改）：
   ```rust
   impl InstanceTemplate {
       /// Native contributions for this template's instances, whichever carrier runs them. For a
       /// spawned daemon they are built again in the daemon's own process.
       pub fn native(mut self, f: impl Fn(&mut EngineContrib) + Send + Sync + 'static) -> Self;
   }
   ```
   用闭包而不是现成的 `EngineContrib`，因为 daemon 进程要自己再建一遍。进程内模板用了它，等同于把结果并进 `contrib`。
2. `daemon_args`（`crates/instance/src/factory.rs:158-183`）多带一个 `--template <id>`（只对声明了 `native` 的模板）。
3. daemon 子命令看到 `--template` 时：照装配的顺序对每个模块调一次 `Module::templates()`（**不**调 `start`，不开任何方法），
   找到这个 id 的模板，用它的 `native` 闭包建 `EngineContrib`，交给 `engine_config`。找不到就以非零退出并写明理由。
4. SDK §6.2 写明边界：spawn 模板的原生贡献跑在 daemon 进程里，**不和模块共享内存**；要模块的状态，经 Harnix 的方法或文件拿。

## 兼容性

- 不调 `native` 的模板：daemon 参数与现在一字不差，行为不变。
- `TemplateCarrier`、`EngineContrib`、`Module` trait 都不改；`MODULE_API_VERSION` 不必升。
- 要求 `Module::templates()` 没有副作用、不依赖 `start` 之后的状态。SDK 现在就是这么描述它的（注册期），写明即可。
- 第三方 `program`（模板指定了别的可执行文件）不受影响：只有用本二进制 daemon 子命令的模板才会带 `--template`。
- Helm、hello-app 不用 spawn 模板，不受影响。

## 验收

- `crates/testapp` 加一个 spawn 模板，`native` 里注册一个原生工具（返回固定字符串）和一个 `customize_session`（在系统提示词末尾加一个标记）。
- `tests/live/api/instances-daemon.spec.ts`：按它建实例、建会话、让模型调那个工具（录像回放），工具结果是那个固定字符串；
  `harnix.system.context`（或会话的上下文读取）里能看到那个标记。
- 同一个模板改成进程内载体，同样的用例通过（两种载体一致）。
- `--template` 指向不存在的 id：daemon 非零退出，实例 `down`，理由可读。

## 依赖

无。Harnix `ipdrun-13`（prompt 中间件）在 spawn 实例上要靠本条才能生效。

## 来源

IPDrun 把一类实例改成 spawn 后，原来进程内的会话定制带不过去，只好给 spawn 那一路另配 command hook 和 `prompt_append`
（`crates/agent/src/config.rs`、`crates/agent/src/hook.rs`），两种载体两套做法。
