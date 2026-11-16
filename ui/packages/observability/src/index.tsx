/**
 * @harnix/observability — the `observability` extension's interface: what a session's recording
 * says, drawn where the session is.
 *
 * - The **trajectory**, a view of the centre beside the conversation (`conversation.view`): every
 *   recorded model call grouped by turn, with a time overview and an inspector.
 * - Two tabs of the session's right column (`sidebar.tab`): **context** (what the next call carries,
 *   the skills and hooks in force, the MCP servers' tools, what trust holds back) and **usage** (this
 *   run's figures, with the way into the trajectory).
 * - The same trajectory as a page of the system space's inspector (`system.inspector.tab`), where
 *   the product has that space, with a way into the session.
 *
 * Read through the host API alone: `observability.*` (the recording), `harnix.session.outline`,
 * `.skills`, `.hooks`, `engine.session.entries` (the feedback), `settings.mcp.context`,
 * `system.context` and `harnix-app.sessions.projectStatus` where the product has them.
 */

import { t, type HostApi, type HostSessionRef as SessionKey, type Registry } from "@harnix/react";

import { ContextTab } from "./context.tsx";
import { TRAJECTORY_VIEW, type Observed } from "./shared.tsx";
import { Trajectory } from "./trajectory/Trajectory.tsx";
import { UsageTab } from "./usage.tsx";
import "./observability.css";

export { summarizeUsage, type UsageSummary } from "./usage.tsx";
export { TRAJECTORY_VIEW };

/** The backend extension this package is the interface of. */
export const extension = "observability";

/** The point the system space defines for pages about a picked session. */
const INSPECTOR_TAB = "system.inspector.tab";

export const CONTEXT_KIND = "context";
export const USAGE_KIND = "usage";

/** What an inspector tab is asked about: a session, and a tool call to pick out. */
interface Target {
  kind: "session";
  instance: string;
  sessionId: string;
  tool?: string;
}

export function register(r: Registry, host: HostApi): void {
  const observed: Observed = { host, registry: r };
  const sessionOf = (target: Target): SessionKey => ({ instance: target.instance, sessionId: target.sessionId });
  r.register(
    "conversation.view",
    {
      id: TRAJECTORY_VIEW,
      order: 10,
      title: () => t("trajectory.title"),
      render: (session) => <Trajectory key={`${session.instance}/${session.sessionId}`} observed={observed} session={session} />,
    },
    extension,
  );
  r.register(
    "sidebar.tab",
    {
      id: "observability.context",
      kind: CONTEXT_KIND,
      priority: "builtin",
      title: () => t("context.title"),
      guide: [{ id: CONTEXT_KIND, title: () => t("context.title"), description: () => t("context.guide"), icon: "context", order: 10 }],
      render: () => <ContextTab observed={observed} />,
    },
    extension,
  );
  r.register(
    "sidebar.tab",
    {
      id: "observability.usage",
      kind: USAGE_KIND,
      priority: "builtin",
      title: () => t("usage.title"),
      guide: [{ id: USAGE_KIND, title: () => t("usage.title"), description: () => t("usage.guide"), icon: "cpu", order: 20 }],
      render: () => <UsageTab observed={observed} />,
    },
    extension,
  );
  // The system space's inspector: the same trajectory, the list giving way to the inspector in the
  // narrow column; a tool row's jump lands on the call that asked for the tool.
  r.register(
    INSPECTOR_TAB,
    {
      id: TRAJECTORY_VIEW,
      tools: true,
      title: () => t("trajectory.title"),
      match: (target: Target) => target.kind === "session",
      render: (target: Target) => (
        <Trajectory key={`${target.instance}/${target.sessionId}/${target.tool ?? ""}`} observed={observed} session={sessionOf(target)} tool={target.tool} compact inSystem />
      ),
    },
    extension,
  );
}
