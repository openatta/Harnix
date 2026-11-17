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
 * - The system space's 用量 tab (`system.console.tab`): every session's figures over the last 30
 *   days by project, day and model (`observability.usage.daily`), with the sections other packages
 *   add to `observability.usage.section`.
 * - Two blocks of the home page (`harnix-app.home.panels`), where the product has the Harnix app:
 *   the process at a glance (`observability.process`, the instances and their Agents, what waits),
 *   and the same usage summary.
 *
 * Read through the host API alone: `observability.*` (the recording), `harnix.session.outline`,
 * `.skills`, `.hooks`, `engine.session.entries` (the feedback), `settings.mcp.context`,
 * `system.context` and `harnix-app.sessions.projectStatus` where the product has them.
 */

import { t, type HostApi, type HostSessionRef as SessionKey, type Registry } from "@harnix/react";

import { ContextTab } from "./context.tsx";
import { GlobalStats, HOME_PANELS, type HomeContext } from "./home.tsx";
import { TRAJECTORY_VIEW, type Observed } from "./shared.tsx";
import { Trajectory } from "./trajectory/Trajectory.tsx";
import { UsageTab } from "./usage.tsx";
import { USAGE_SECTION, UsageSummaryTab } from "./summary.tsx";
import "./observability.css";

export { summarizeUsage, type UsageSummary } from "@harnix/core";
export { USAGE_SECTION, type UsageSection, type UsageSectionContext } from "./summary.tsx";
export { TRAJECTORY_VIEW };
export { GlobalStats, agentActive, type ProcessFigures } from "./home.tsx";

/** The backend extension this package is the interface of. */
export const extension = "observability";

/** The points the system space defines for pages about a picked session and for its console's tabs. */
const INSPECTOR_TAB = "system.inspector.tab";
const CONSOLE_TAB = "system.console.tab";

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
  // The 用量 tab's sections, for other packages to add theirs (the flows, by flow).
  r.definePoint(USAGE_SECTION, { kind: "render" });
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
  // The home page: the process at a glance first, the usage of the last days after the lists.
  // Where there is no Harnix app there is no home page, and neither block.
  r.register(
    HOME_PANELS,
    { id: "observability.stats", order: 10, wide: true, title: () => t("home.stats.title"), render: (context: HomeContext) => <GlobalStats host={host} context={context} /> },
    extension,
    { orElse: () => undefined },
  );
  r.register(
    HOME_PANELS,
    { id: "observability.summary", order: 60, wide: true, title: () => t("usageAll.title"), render: () => <UsageSummaryTab observed={observed} /> },
    extension,
    { orElse: () => undefined },
  );
  // The system space's console: what every session has used over the last days. Where there is no
  // system space there is no console, and no tab.
  r.register(
    CONSOLE_TAB,
    {
      id: "observability.summary",
      order: 40,
      title: () => t("usageAll.title"),
      about: () => t("usageAll.about"),
      render: () => <UsageSummaryTab observed={observed} />,
    },
    extension,
    { orElse: () => undefined },
  );
}
