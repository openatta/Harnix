/**
 * The flow module's interface as a UI package: what the shell activates while the backend's `flow`
 * extension is enabled.
 *
 * The flows are a space of their own (`space.tsx`), at the bottom of the left column: the
 * application's toolchain, managed in one place.
 *
 * What the runs spent, by flow, is a section of the usage summary (`observability.usage.section`),
 * where that point is defined.
 *
 * And a task — a run in a project: the kind `flow.task` of the projects space's
 * `harnix-app.project.items` (`task.tsx`), where that point is defined, and the right column's
 * 流程 tab of a task's main session (`TaskTab.tsx`), a `sidebar.tab` of kind `flow`. And
 * `/flow` in a conversation, with the card it leaves there, and "为这个项目建流程" in a project's
 * menu (`slash.tsx`).
 *
 * It defines two points of its own (`points.tsx`): a badge on a task's row, and another way to start
 * a flow than now — what a schedule's interface fills.
 */

import { t, type HostApi, type Registry } from "@harnix/react";

import { START_ACTION, TASK_BADGE } from "./points";
import { registerFlowSpace, type FlowSpace } from "./space";
import { registerSlashFlow, registerTaskCard } from "./slash";
import { openTask, registerTaskItems, TASK_TAB_KIND } from "./task";
import { TaskTab } from "./TaskTab";
import { UsageByFlow } from "./UsageByFlow";

/** The backend extension this is the interface of. */
export const extension = "flow";

/** The usage summary's sections, where the point is: one per flow. */
const USAGE_SECTION = "observability.usage.section";
/** The backend space the flows' conversations run in (the module's default; an application that
 *  renames it registers `registerFlowSpace` itself). */
const SPACE = "flow";
const MODULE = "flow";

export function register(r: Registry, host: HostApi): void {
  // A task row's badges and a flow's other ways to start, for other packages to fill (`points.tsx`).
  r.definePoint(TASK_BADGE, { kind: "render" });
  r.definePoint(START_ACTION, { kind: "register" });
  const space: FlowSpace = registerFlowSpace(r, { space: SPACE, host });
  // A task's main session: the run in the right column. Kept mounted: the graph's pick survives a switch.
  r.register(
    "sidebar.tab",
    {
      id: "flow.task",
      kind: TASK_TAB_KIND,
      keepMounted: true,
      title: () => t("flowui.task.tab"),
      render: () => <TaskTab host={host} module={MODULE} />,
    },
    extension,
  );
  const tasks = registerTaskItems(r, host, MODULE, () => space, extension);
  // `/flow <flow> [subject]` in a conversation of a project, and the card it leaves there.
  registerSlashFlow(r, host, MODULE, () => tasks.all(), extension);
  registerTaskCard(
    r,
    extension,
    MODULE,
    async (card) => {
      if (space) await openTask(host, r, space, card.run, card.flow);
    },
    extension,
  );
  // The usage summary (the observability package's) lists what the runs spent, by flow; without it, nothing.
  r.register(
    USAGE_SECTION,
    {
      id: "flow.usage",
      order: 10,
      title: () => t("flowui.usage.title"),
      render: (context: { since: number }) => <UsageByFlow module={MODULE} since={context.since} />,
    },
    extension,
    { orElse: () => undefined },
  );
}
