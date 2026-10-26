/**
 * @harnix/core —— 不依赖框架的那一层（设计 §4.2）。
 *
 * 子路径各对应后端的一层：`/protocol` ↔ `harnix-contract`，`/client` ↔ `harnix-transport`，
 * `/state` ↔ 中枢推出的事件与整值快照。根入口把它们一起导出，方便不在意分层的调用方。
 */

export * from "./protocol/index.ts";
export * from "./client/index.ts";
