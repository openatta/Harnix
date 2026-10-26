/**
 * `@harnix/core/protocol` —— 协议的类型与常量。
 *
 * 两层：`generated/` 由 `harnix-contract` 经 ts-rs 生成（版本号、错误码、握手、主体、通道），
 * 不手改，`scripts/check.sh` 比对漂移；`frames.ts` 是方法与事件的形状，手写，以中枢与 Agix
 * 协议为准，只写客户端读到的字段，其余以索引签名放行。
 */

export * from "./generated/contract.ts";
export * from "./frames.ts";
