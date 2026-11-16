/**
 * `@harnix/core/protocol` —— 协议的类型与常量。
 *
 * 三层：`generated/` 从 Rust 生成（`crates/harnix/tests/ts_export.rs`）——版本号、错误码、握手，
 * `harnix.hello`、中枢的帧与行（`harnix_contract::wire`），录像读出来的形状（`observability.*` 的答），
 * `packages.list` 的行与清单的 `[inline]`（`harnix_ext_packages::wire`）；
 * 不手改，`scripts/check.sh` 比对漂移。`engine.ts` 是引擎（Agix）的形状，手写，照 `hello.engine_protocol`。
 * `frames.ts` 把两者接到方法名与通知名上。
 */

export * from "./generated/contract.ts";
export * from "./generated/readers.ts";
export * from "./generated/packages.ts";
export * from "./engine.ts";
export * from "./frames.ts";
export * from "./events.ts";
export * from "./changes.ts";
export * from "./feedback.ts";
