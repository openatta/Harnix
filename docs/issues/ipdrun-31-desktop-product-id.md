# ipdrun-31：桌面模板在没有 profile 时把状态目录写死成 `~/.harnix`

类型：缺陷｜优先级：中

## 现象

桌面模板的壳在本地模式下读应用写的令牌，令牌在应用的状态目录里。目录由 `state_dir`（`templates/desktop/src-tauri/src/local.rs:234-251`）推出：
`HARNIX_STATE_DIR` → profile 里的 `[brand] product_id` → 兜底 `"harnix"`。

应用把产品 profile 嵌在二进制里、壳不传 `--profile` 时（模板的缺省就是不传），壳读不到 product id，落到 `~/.harnix/token`；应用写的却是
`~/.<product_id>/token`。窗口和 `--smoke` 都等满 60 秒后报「读不到 …/.harnix/token」。

改名清单（`templates/desktop/README.md:129-` 的「改名」一节）列了 `ENV_PREFIX`、`SIDECAR`，没有这一处；「已知的限制」最后一条（`README.md:223-224`）写着壳要镜像
应用的路径规则。

## 建议

1. 兜底的产品 id 做成和 `ENV_PREFIX`、`SIDECAR` 并列的常量（`templates/desktop/src-tauri/src/config.rs:21-28` 一带）：
   ```rust
   /// `~/.<PRODUCT_ID>` when neither `HARNIX_STATE_DIR` nor a profile says otherwise. An app renames it with everything else.
   pub const PRODUCT_ID: &str = "harnix";
   ```
   `state_dir` 的 `unwrap_or_else(|| "harnix".to_string())` 换成它；README「改名」一节加一行。
2. 更好的一步（可后做）：壳不再推目录，而是问子进程。例如应用二进制支持 `--print-state-dir`（只读、立即退出），或就绪前在 stdout 打一行
   约定格式的状态目录；壳先用它，拿不到再按 1 推。这样就去掉了 README 里「镜像路径规则」那条限制。

## 兼容性

- 1：常量缺省就是 `"harnix"`，没改名的应用行为不变；已经拷了模板的应用不受影响，下次同步模板时多改一处。
- 2：新开关；旧的应用二进制不认识它时报错退出，壳退回 1 的推法。
- 只动模板，Harnix 的 crate 与协议不变。

## 验收

- `tests/app-template.mjs`（或模板自己的检查）：`PRODUCT_ID` 存在，`state_dir` 不再含字面量 `"harnix"`；README 的改名清单列了它。
- 若做 2：`harnix --print-state-dir` 输出 `~/.harnix`（或 `HARNIX_STATE_DIR`），带 `[brand] product_id = "x"` 的 profile 输出 `~/.x`。

## 依赖

无。

## 来源

IPDrun 的二进制内嵌 profile、壳不传 `--profile`，桌面版启动就卡在读令牌上；IPDrun 在自己的拷贝里把兜底改成了 `"ipdrun"`（`desktop/src-tauri/src/local.rs`）。
