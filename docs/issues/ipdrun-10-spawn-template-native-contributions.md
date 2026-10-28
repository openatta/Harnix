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

## 实现记录

（F3c，与 ipdrun-07、ipdrun-13 同一提交，提交号见 `git log --grep ipdrun-10`。）

- `InstanceTemplate::native(f)`（字段 `native: Option<NativeContrib>`，`NativeContrib = Arc<dyn Fn(&mut EngineContrib) + Send + Sync>`）与
  `native_contrib()`。`Factory::build_template`：进程内把它建出来并在 `contrib` 之后（`extend`）；spawn 且 `program` 为 `None` 时 daemon 参数多一个
  `--template <模块>.<模板>`（没写 `native` 的模板参数一字不差）。
- daemon 子命令（`crates/app/src/run.rs`）看到 `--template`：`modules::native_of` 按添加顺序对每个**应用**模块只调 `manifest()` 与 `templates()`，
  找到这个全名、调闭包，交给新增的 `serve_daemon_with(args, contrib)`（`serve_daemon` 不变，等于给空的）。找不到（或模板没有 `native`）就
  `refuse_daemon`：理由写到 socket 旁的 `<socket>.refused`、stderr 也写一行，以 `REFUSED_EXIT`（78）退出。`DaemonLink` 看到 daemon 在监听前以 78
  退出，读那个文件，实例 `down`（理由 `the daemon refused to start (…): …`），不再重启。
- SDK §6.2 写明：闭包在 daemon 进程里再建一遍、不和模块共享内存；`templates()` 要无副作用；贡献的场景要列进 `EngineProfile.scenes` 才启用。
- 测试：`tests/live/api/native-contrib.spec.ts` 的「spawn: the template's native tool answers and its block is in the system prompt」与
  「in_process: …」同一份用例（testapp `fleet.native-spawn` / `fleet.native-local`，同一个 `native`：场景 `probe-native` 带工具 `ProbeFixed`、
  `customize_session` 在系统提示词末尾加一个标记块）——工具结果是那个固定字符串，`session.inspectContext`（经 `app.fleet.context`）里有标记；
  「a daemon asked for a template this binary does not declare refuses to start, saying so」（直接起 `harnix-testapp daemon --template fleet.nowhere`：
  退出码 78、理由文件、没监听）；Rust `daemon_link::tests::a_daemon_that_refuses_what_it_was_started_with_takes_the_instance_down`（link 那一半：实例
  `down`、理由可读）。新录像 `calls-a-native-tool`（由 `calls-a-tool` 手改）。

偏离：

- 用例放在新文件 `native-contrib.spec.ts`（起自己的 testapp 后端），没放进 `instances-daemon.spec.ts`。
- 「`--template` 指向不存在的 id → 实例 down」拆成两半测：daemon 退出码与理由（直接起二进制），link 收到后 `down`（Rust 用假 daemon）。正常装配下
  同一个二进制不会找不到自己的模板，所以没有经中枢的端到端用例。
- 内建模块不参与查找（它们不声明模板）。
- 测试里的原生工具是经场景的 `extra_tools` 加的：Agix 的 `Builder` 没有加单个工具的方法（SDK §6.1 / §6.2 示例里的 `b.native_tool(…)` 不存在，未改）。
