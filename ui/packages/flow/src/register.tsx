/**
 * The flow module's interface as a UI package: what the shell activates while the backend's `flow`
 * extension is enabled.
 *
 * The flows are an application's toolchain, so where a console of the process is defined
 * (`system.console.tab`) they are a tab of it, and a flow clicked there is a space shown inside the
 * console's space. Where nobody defines that point, they are a space of their own (`orElse`). Which
 * one is decided by whether the point is defined, not by which extensions run.
 *
 * What the runs spent, by flow, is a section of the system space's 用量 tab
 * (`observability.usage.section`), where that point is defined.
 *
 * And a task — a run in a project: the kind `flow.task` of the projects space's
 * `harnix-app.project.items` (`task.tsx`), where that point is defined, and the right column's
 * 流程 tab of a task's main session (`TaskTab.tsx`), a `sidebar.tab` of kind `flow`. And
 * `/flow` in a conversation, with the card it leaves there, and "为这个项目建流程" in a project's
 * menu (`slash.tsx`).
 */

import { t, type HostApi, type Registry } from "@harnix/react";

import { registerFlowSpace, registerSystemFlows, type FlowSpace } from "./space";
import { registerSlashFlow, registerTaskCard } from "./slash";
import { openTask, registerTaskItems, TASK_TAB_KIND } from "./task";
import { TaskTab } from "./TaskTab";
import { UsageByFlow } from "./UsageByFlow";

/** The backend extension this is the interface of. */
export const extension = "flow";

/** The point a console tab goes into, and the backend space the flows' conversations run in (the
 *  module's default; an application that renames it registers `registerFlowSpace` itself). */
const CONSOLE = "system.console.tab";
/** The system space's 用量 tab, where it is: a section of it per flow. */
const USAGE_SECTION = "observability.usage.section";
const SPACE = "flow";
const MODULE = "flow";

export function register(r: Registry, host: HostApi): void {
  const tab = { id: "flow.flows", order: 20, title: () => t("flowui.tab.flows"), about: () => t("flowui.about.flows") };
  // The flow space, once registered: inside the console now, or of its own once every package has registered.
  let space: FlowSpace | null = null;
  // The console is there already: the flows go inside the space that defines it.
  const definer = r.definitions().find((d) => d.name === CONSOLE)?.owner;
  if (definer !== undefined) {
    const view = registerSystemFlows(r, { space: SPACE, within: definer, host });
    space = view;
    r.register(CONSOLE, { ...tab, render: view.render }, extension);
  } else {
    // Not yet: if it is by the time every package has registered, the tab is drawn empty (its space
    // came after this one); if it never is, the flows are a space of their own.
    r.register(CONSOLE, { ...tab, render: () => null }, extension, {
      orElse: () => {
        space = registerFlowSpace(r, { space: SPACE, host });
      },
    });
  }
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
  // The 用量 tab (the observability package's) lists what the runs spent, by flow; without it, nothing.
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
