/**
 * @harnix/observability — the `observability` extension's interface: what a session's recording
 * says, and what the engine holds of it now, drawn where the session is.
 *
 * - The **trajectory**, a view of the centre beside the conversation (`conversation.view`): every
 *   recorded model call grouped by turn, with a time overview and an inspector.
 * - Three tabs of the session's right column (`sidebar.tab`): **context** — standing at the end of
 *   every session's strip (`pinned`): what the next call carries, block by block with its source
 *   and size, the tools, the skills and hooks in force, the MCP servers' tools, what trust holds
 *   back, the model; **usage** (this run's figures, with the way into the trajectory); and
 *   **output**, one tab per background command or monitor, opened from the tool's row, with the way
 *   to stop it while it runs (`observability.tasks.stop`).
 * - A person's message's menu (`session.action` at `message`): rewrite it, or take it out of the
 *   context (`observability.messages.edit`, `.drop`).
 * - Two blocks of the home page (`harnix-app.home.panels`), where the product has the Harnix app:
 *   the process at a glance (`observability.process`, the instances and their Agents, what waits),
 *   and the usage summary (`UsageSummaryTab`: every session's figures over the last 30 days by
 *   project, day and model, `observability.usage.daily`, with the sections other packages add to
 *   `observability.usage.section`).
 * - A block of the settings' diagnostics (`settings.section` `within: "diagnostics"`): each
 *   extension's log (`observability.extensions.log`), filtered, followed live, copied.
 *
 * Read through the host API alone: `observability.*` (the recording, the context, a task's
 * output and stopping it, a message changed, the process, the extensions' log), `harnix.session.outline`, `.skills`, `.hooks`,
 * `.tasks`, `engine.session.entries` (the feedback), `settings.mcp.context` and
 * `harnix-app.sessions.projectStatus` where the product has them.
 */

import { t, type ActionTarget, type HostApi, type Registry } from "@harnix/react";

import { ContextTab } from "./context.tsx";
import { EXTLOG, ExtensionLogView } from "./extlog.tsx";
import { GlobalStats, HOME_PANELS, type HomeContext } from "./home.tsx";
import { registerMessageActions } from "./messages.tsx";
import { OUTPUT_KIND, OUTPUT_TOOLS, OutputTab, openToolOutput, outputTitle } from "./output.tsx";
import { TRAJECTORY_VIEW, type Observed } from "./shared.tsx";
import { Trajectory } from "./trajectory/Trajectory.tsx";
import { UsageTab } from "./usage.tsx";
import { USAGE_SECTION, UsageSummaryTab } from "./summary.tsx";
import "./observability.css";

export { summarizeUsage, type UsageSummary } from "@harnix/core";
export { USAGE_SECTION, UsageSummaryTab, type UsageSection, type UsageSectionContext } from "./summary.tsx";
export { SessionContext } from "./context.tsx";
export { OUTPUT_KIND, outputAddress, parseOutputAddress, stripAnsi, taskLabel } from "./output.tsx";
export { TRAJECTORY_VIEW };
export { GlobalStats, agentActive, type ProcessFigures } from "./home.tsx";
export { EXTLOG, ExtensionLogView, lineText, merged } from "./extlog.tsx";

/** The backend extension this package is the interface of. */
export const extension = "observability";

export const CONTEXT_KIND = "context";
export const USAGE_KIND = "usage";

export function register(r: Registry, host: HostApi): void {
  const observed: Observed = { host, registry: r };
  // The usage summary's sections, for other packages to add theirs (the flows, by flow).
  r.definePoint(USAGE_SECTION, { kind: "render" });
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
  // The context stands at the end of every session's strip: the one place the whole of the next
  // call is seen.
  r.register(
    "sidebar.tab",
    {
      id: "observability.context",
      kind: CONTEXT_KIND,
      priority: "builtin",
      pinned: true,
      title: () => t("context.title"),
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
  // The output of a background command or a monitor, in the open session's right column: one tab a task.
  r.register(
    "sidebar.tab",
    {
      id: "observability.output",
      kind: OUTPUT_KIND,
      priority: "builtin",
      multiple: true,
      keepMounted: true,
      title: (address?: string) => outputTitle(address, t("obs.output.title")),
      render: () => <OutputTab observed={observed} />,
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
  // Each extension's log, in the settings' diagnostics.
  r.register("settings.section", { id: EXTLOG, within: "diagnostics", label: () => t("obs.extlog"), render: () => <ExtensionLogView host={host} /> }, extension);
  r.register(
    "session.action",
    {
      id: "observability.output",
      at: ["tool"],
      icon: "terminal",
      label: () => t("obs.output.view"),
      match: (target: ActionTarget) => target.kind === "tool" && OUTPUT_TOOLS.has(target.tool),
      run: async (target: ActionTarget) => {
        if (target.kind !== "tool") return;
        if (!(await openToolOutput(host, target.session, target.toolUseId))) host.shell.notice(t("obs.output.noTask"));
      },
    },
    extension,
  );
  registerMessageActions(r, host, extension);
}
