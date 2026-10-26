import { useConnection, useStrings } from "../context";

/** 连接状态：已连接 / 连接中 / 已断开 / 版本不兼容（带理由）。 */
export function ConnectionBadge() {
  const strings = useStrings();
  const { status, detail } = useConnection();
  const label =
    status === "open" || status === "degraded"
      ? strings.connOpen
      : status === "incompatible"
        ? strings.connIncompatible
        : status === "connecting"
          ? strings.connConnecting
          : strings.connClosed;
  return (
    <span
      className={`hx-conn hx-conn-${status}`}
      data-testid="conn-status"
      data-state={status}
      role="status"
      title={detail}
    >
      <span className="hx-dot" aria-hidden />
      <span>{label}</span>
    </span>
  );
}
