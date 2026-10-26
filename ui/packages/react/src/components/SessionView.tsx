/**
 * 会话视图：通用会话壳用的 ChatPanel 配置（抬头写会话名、用量、连接）。
 *
 * 挂到一个会话上（harnix.attach）就开始追赶：历史、中枢缓冲区的重放、挂起的提问，然后实时。
 * 换会话或卸载时 detach。要别的样子（角色、面包屑、快捷回复、转录……）直接用 ChatPanel。
 */

import type { ReactNode } from "react";
import { ChatPanel } from "../chat/ChatPanel";
import type { ToolRowContribution, UserRowContribution } from "./blocks";

export interface SessionViewProps {
  sessionId: string;
  /** 工具行的注册表（Nest 的 tool.row 贡献点）：后注册的先匹配，没人认领用内建的。 */
  toolRows?: readonly ToolRowContribution[];
  /** 人的消息的注册表：后注册的先匹配，没人认领用内建的气泡。 */
  userRows?: readonly UserRowContribution[];
  /** 抬头右侧的附加内容。 */
  headerExtra?: ReactNode;
  /** 输入区上方的附加内容（例如发送前检查的确认卡）。 */
  composerTop?: ReactNode;
  /** 代替 `harnix.send` 的发送（ChatPanelProps.send）。 */
  onSend?: (text: string) => Promise<boolean>;
}

export function SessionView({ sessionId, toolRows, userRows, headerExtra, composerTop, onSend }: SessionViewProps) {
  return (
    <ChatPanel
      sessionId={sessionId}
      header={{ title: true, role: false, usage: true, connection: true, actions: headerExtra }}
      guard={composerTop}
      send={onSend}
      renderers={{ toolRows, userRows }}
      features={{ contextRing: false }}
    />
  );
}
