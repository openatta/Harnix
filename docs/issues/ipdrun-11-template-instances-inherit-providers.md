# ipdrun-11：模板建的实例读不到「设置 → 模型」里配的 provider

类型：新能力｜优先级：中

## 现象

设置页的 provider 由 `harnix.settings.setProvider` 写进 **`system` 实例**数据根的全局层（`crates/builtin/src/settings.rs:264-293`，
路径来自 `tier_path("global")` = `<system data_root>/settings.json`，`:296-309`），然后只对 `system` 发 `config.reload`。

按模板建的实例各有自己的数据根（`<状态目录>/instances/<data_root>`），引擎装配时全局层就是它自己的根
（`crates/instance/src/engine.rs:144-148` 的 `StaticDaemonPaths::with_global(config.data_root)`）。所以：

1. 设备在设置页加了一个 provider；
2. `config.getProvider {instance: "system"}` 有它；
3. 同一个进程里按模板建的实例 `config.getProvider` 是 `{"providers":{}}`，在那台实例里开的会话没有模型可用。

每个用模板实例的应用都得自己把 system 的 provider 抄进各实例的数据根，再逐台 `config.reload`，而且只能在「下次建会话前」抄，
设置页改了以后不是即时的。

## 建议

1. **模板声明继承**（只加不改）：
   ```rust
   impl InstanceTemplate {
       /// This template's instances see the providers configured for `system` (the settings page's), under their own.
       pub fn inherit_providers(mut self) -> Self;
   }
   ```
   profile 里声明的模板同样可写 `inherit_providers = true`。
   声明了的实例：建成时、以及每次 `harnix.settings.setProvider` 成功后，Harnix 把 system 全局层的 `providers` / `default_provider`
   写进这台实例的一个专用层（例如 `<实例数据根>/settings.inherited.json`，0600），再对它 `config.reload`。实例自己全局层里的同名 provider 优先。
   若 Agix 的设置层能直接多叠一层只读的「上游全局」，就不必复制文件；那是 Agix 的事，这里先用复制做到。
2. **变更通知**：模块要在 provider 之上叠自己的配置时，需要知道它变了。
   ```rust
   impl ModuleContext {
       /// Bumped after every successful settings write of the given scope.
       pub fn settings_changes(&self) -> tokio::sync::watch::Receiver<u64>;
   }
   ```
   界面侧可以有一个宿主主题 `harnix.settings`（整值快照：`{revision, providers: [id…], default_provider}`，不含密钥），设置页之外的
   space 也能跟着刷新。
3. `setProvider` 的返回里加 `reloaded_instances: [id…]`，说清这次推到了哪些实例。

## 兼容性

- 不声明 `inherit_providers` 的模板：行为不变，实例照旧只读自己的数据根。
- `system` 实例不受影响。
- `setProvider` 返回多一个字段，旧调用方忽略即可。
- 新主题、新方法都是新增；协议版本不必升。
- 密钥只复制进 0600 的文件，不进主题、不进日志。
- Helm、hello-app 只用 `system`，不受影响。

## 验收

- `tests/live/api/settings.spec.ts`：testapp 声明一个 `inherit_providers` 的进程内模板。建实例 → 设备 `setProvider "hx-check"` →
  这台实例的 `config.getProvider` 里有 `hx-check`（不用重建实例）；`setProvider {delete: true}` 后它消失。
- 不声明的模板：`config.getProvider` 仍为空（回归）。
- spawn 载体同样一遍（`instances-daemon.spec.ts`）。
- 模块侧：`settings_changes()` 在 `setProvider` 后 1 s 内变化。

## 依赖

无（复制的做法只用 Harnix 已有的 `config.reload`）。

## 来源

IPDrun 的模块按模板建的实例读不到设置页配的 provider；IPDrun 现在在每次建 / 恢复会话前把 system 的 provider 与自己的合起来写进
实例数据根再 `config.reload`（`crates/agent/src/host.rs` 的 `sync_providers`），设置页改了以后不是即时的。
