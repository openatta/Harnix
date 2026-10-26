/**
 * `@harnix/core/state` —— 一个打开的会话的状态：归约成块的对话流、队列、待送达，以及驱动它的控制器。
 * 不依赖框架；React 层用 `useSyncExternalStore` 订阅控制器（设计 §4.2，M4-1）。
 */

export * from "./model.ts";
export { SessionController, SYSTEM_INSTANCE, type SessionControllerOptions, type SessionStrings } from "./controller.ts";
