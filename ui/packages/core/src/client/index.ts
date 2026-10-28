/**
 * `@harnix/core/client` —— 协议客户端：握手、拓扑与回退、call / notify / answer、连接状态。
 * 不依赖框架；令牌、基址、WebSocket / fetch / EventSource 都由构造参数注入，node 与测试里也能跑。
 */

export {
  HarnixClient,
  SPEAKS,
  TOKEN_STALE,
  errorText,
  tokenFromDocument,
  tokenFromPage,
  type ConnectionStatus,
  type HarnixClientOptions,
  type NotifyListener,
  type StatusListener,
  type WebSocketCtor,
  type WebSocketLike,
} from "./client.ts";
export {
  connectSplit,
  type EventSourceCtor,
  type EventSourceLike,
  type FetchLike,
  type IncomingFrame,
  type SplitCarrier,
} from "./split.ts";
