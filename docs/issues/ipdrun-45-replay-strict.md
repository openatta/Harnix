# ipdrun-45：回放只能非严格

类型：新能力｜优先级：中

## 需求

`crates/hub/src/lib.rs` 的 `with_recording` 把回放写死成 `strict: false`：请求与录下的不一样只报告。理由写在那里——组装的提示里有今天的日期与
绝对路径，严格回放第二天就红，要等引擎的 `environment` 接缝能由宿主设定。应用于是只能数发散、给个阈值，录像已经不再代表现在的提示了也看不出来。

## 建议（只加不改）

- Agix ipdrun-06：回放的会话从录像开录的时刻起记时间（提示里的日期与录下来的那天相同）；`options.clock` 也给宿主自己定起点。
- `harnix serve --replay-strict`（要和 `--replay-dir` 一起给）：`with_recording` 写 `strict: true`，不一样的那次调用失败。仍是启动进程的人定的，
  客户端改不了（与回放本身同一个道理）。`Hub::set_replay_strict(bool)`，`HubConfig` 不变。
- testkit `startHarnix({ replay: true, replayStrict: true })`。
- 路径：应用自己保持稳定（固定根目录；文件工具的相对路径见 Agix grabix-01 `path_style`）。

## 兼容性

不加 `--replay-strict`：同现在（非严格）。回放会话的日期变成录像那天（Agix ipdrun-06），非严格回放的发散只会变少。

## 验收

`crates/hub` 单测 `replay_is_lenient_unless_the_operator_asked_for_strict`：缺省 `strict: false`、开了 `true`、客户端自带的 mode / strict 被覆盖、
不在回放时没有 strict。日期由 Agix ipdrun-06 的单测覆盖。

## 依赖

Agix ipdrun-06（agix 子模块指到它之后，日期才按录像定）。

## 来源

IPDrun 的回放档（stage1–4 的 API 层）只能非严格、按发散数阈值判（R9 时 31 / 0 / 3 / 10，上限 40）。
