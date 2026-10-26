/**
 * 会话列表（展示组件）。数据由调用方取（`harnix.sessions` 是整值快照），这里只负责画、
 * 选中与"新会话"。对应 旧界面 ui/shell/sidebar.js 的最小部分（不含分组、归档、搜索）。
 */

import type { SessionRow } from "@harnix/core";
import { useStrings } from "../context";

export interface SessionListProps {
  sessions: readonly SessionRow[];
  selected: string | null;
  onSelect(sessionId: string): void;
  onNew?: () => void;
  /** 正在创建新会话。 */
  creating?: boolean;
}

export function SessionList({ sessions, selected, onSelect, onNew, creating }: SessionListProps) {
  const strings = useStrings();
  return (
    <nav className="hx-sessions" aria-label={strings.sessions} data-testid="session-nav">
      {onNew ? (
        <button
          type="button"
          className="hx-btn hx-btn-primary hx-new"
          data-testid="session-new"
          disabled={creating}
          onClick={onNew}
        >
          ＋ {strings.newSession}
        </button>
      ) : null}
      {sessions.length === 0 ? (
        <div className="hx-placeholder" data-testid="session-list-empty">
          {strings.noSessions}
        </div>
      ) : (
        <ul className="hx-session-list" data-testid="session-list" role="list">
          {sessions.map((session) => (
            <li key={session.session_id}>
              <button
                type="button"
                className="hx-session-row"
                data-testid="session-row"
                data-id={session.session_id}
                data-state={session.running ? "running" : "idle"}
                aria-current={session.session_id === selected ? "true" : undefined}
                onClick={() => onSelect(session.session_id)}
              >
                <span className="hx-session-name hx-truncate">
                  {session.name || session.preview || strings.untitled}
                </span>
                {session.running ? <span className="hx-spinner" aria-hidden /> : null}
              </button>
            </li>
          ))}
        </ul>
      )}
    </nav>
  );
}
