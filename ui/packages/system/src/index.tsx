/**
 * @harnix/system — the system space (concept §5.5): what this process runs, and the hands to act on it.
 *
 * Its three columns: on the left, the instances; in the centre, the console — the process at a
 * glance, then the Agents (the active ones, or all of them: each main Agent on a row with its
 * sub-agents, tasks, tokens and model time, opened into its sub-agents and tasks), then the tabs
 * other packages put on it (`system.console.tab`: the flows, the extensions); on the right,
 * whatever was picked there: an instance's facts and its restart, or a session's facts and
 * operations, the context the next call will carry, its tasks, its frames as they arrive, and the
 * tabs other packages put there (`system.inspector.tab`: every model call, each turn). There is no
 * conversation here: the system space looks at sessions, it does not hold one.
 *
 * The interface of the `system` extension, registered while it is enabled, and built on the host
 * API alone: `system.*` for what the engine keeps behind that extension, `harnix.session.*` and
 * `harnix.instances.*` claiming the system space for the rest — every one of them audited.
 */

import { Contributed, t, type ActionTarget, type HostApi, type HostSessionRef as SessionKey, type Registry, type SpaceTab } from "@harnix/react";

import { CONSOLE_TAB, INSPECTOR_TAB, SYSTEM } from "./constants.ts";
import { Console, ConsoleAbout, Drawn } from "./console.tsx";
import { InstanceList } from "./instances.tsx";
import { ContextTab } from "./pane/context.tsx";
import { FramesTab } from "./pane/frames.tsx";
import { InstanceOverview } from "./pane/instance.tsx";
import { SessionOverview } from "./pane/session.tsx";
import { SpaceSessions } from "./pane/space.tsx";
import { SubAgentOverview } from "./pane/subagent.tsx";
import { TasksTab } from "./pane/tasks.tsx";
import type { InspectorTab, InspectorTarget } from "./types.ts";
import { View, type Selection } from "./view.ts";
import "./system.css";

export type { ConsoleTab, InspectorTab, InspectorTarget } from "./types.ts";
export type { Selection } from "./view.ts";
export { SYSTEM, CONSOLE_TAB, INSPECTOR_TAB };

/** The backend extension this package is the interface of. */
export const extension = "system";

/**
 * Into the system space from a conversation. With `tool`: that session, the tab that shows its
 * tool calls opened on the call that asked for the tool. Without: the sessions of the space the
 * person came from, theirs first — an application has many, and the one in front is only one of them.
 */
async function openInSystem(view: View, target: { session: SessionKey; space: string; tool?: string }): Promise<void> {
  await view.host.spaces.switch(SYSTEM);
  if (target.tool) {
    view.select({ kind: "session", ...target.session, tool: target.tool });
    const tab = inspectorTabs(view, { kind: "session", ...target.session, tool: target.tool }).find((x) => x.tools);
    if (tab) view.host.details.open(tab.id);
  } else {
    view.select({ kind: "space", space: target.space, current: target.session });
  }
}

export function register(r: Registry, host: HostApi): void {
  r.definePoint(CONSOLE_TAB, { kind: "register" });
  r.definePoint(INSPECTOR_TAB, { kind: "render" });
  const view = new View(host, r);
  r.register(
    "space",
    {
      id: SYSTEM,
      icon: "cpu",
      order: 90,
      // It is about the process, not one of the product's surfaces: at the bottom of the left column.
      place: "bottom",
      title: () => t("space.system"),
      conversation: { subtitle: () => t("space.system.subtitle") },
      center: { mode: "hidden", overview: () => <Console view={view} /> },
      sidebar: {
        render: () => <InstanceList view={view} />,
        // How many sessions run a turn now, on its entry.
        badge: () => view.running().size || null,
      },
      details: { width: 0.45, tabs: () => tabsFor(view) },
      // Drawn again when what the badge counts changes.
      store: { subscribe: (fn) => view.watch("harnix.sessions.status", {}, fn), getSnapshot: () => view.running().size },
    },
    SYSTEM,
  );
  // Into this space from a conversation elsewhere, or from one tool call in it.
  r.register(
    "session.action",
    {
      id: "system.open",
      at: ["header"],
      icon: "cpu",
      label: () => t("head.inSystem"),
      match: (target: ActionTarget) => target.space !== SYSTEM,
      run: (target: ActionTarget) => openInSystem(view, { session: target.session, space: target.space }),
    },
    SYSTEM,
  );
  r.register(
    "session.action",
    {
      id: "system.tool",
      at: ["tool"],
      label: () => t("flow.inSystem"),
      match: (target: ActionTarget) => target.kind === "tool",
      run: (target: ActionTarget) => openInSystem(view, { session: target.session, space: target.space, tool: target.kind === "tool" ? target.toolUseId : undefined }),
    },
    SYSTEM,
  );
}

/** The inspector tabs other packages put on a session, that say they apply to it. */
function inspectorTabs(view: View, target: InspectorTarget): (InspectorTab & { owner: string })[] {
  return view.registry.list<InspectorTab>(INSPECTOR_TAB).filter((tab) => {
    try {
      return tab.match(target) === true;
    } catch (e) {
      queueMicrotask(() => view.registry.fail(INSPECTOR_TAB, tab, e));
      return false;
    }
  });
}

/** An inspector tab as a tab of the right column: its title and render guarded, each failing on its own. */
function asSpaceTab(view: View, tab: InspectorTab & { owner: string }, target: InspectorTarget): SpaceTab {
  return {
    id: tab.id,
    title: () => {
      try {
        return tab.title(target);
      } catch {
        return tab.id;
      }
    },
    render: () => (
      <Contributed registry={view.registry} point={INSPECTOR_TAB} contribution={tab}>
        <Drawn render={() => tab.render(target)} />
      </Contributed>
    ),
  };
}

/** The right column's tabs for what is selected: only the ones that apply. */
function tabsFor(view: View): SpaceTab[] {
  const picked: Selection | null = view.selected();
  if (!picked) return [{ id: "system.about", title: () => t("system.tab.about"), render: () => <ConsoleAbout view={view} /> }];
  if (picked.kind === "space") {
    return [{ id: "system.overview", title: () => t("system.tab.spaceSessions"), render: () => <SpaceSessions view={view} space={picked.space} current={picked.current ?? null} /> }];
  }
  if (picked.kind === "instance") {
    return [{ id: "system.overview", title: () => t("system.tab.overview"), render: () => <InstanceOverview view={view} instance={picked.instance} /> }];
  }
  const session: SessionKey = { instance: picked.instance, sessionId: picked.sessionId };
  // A sub-agent's calls are in its parent's recording and its runtime state in its parent's tasks.
  if (picked.kind === "session" && picked.parent) {
    const parent = picked.parent;
    return [{ id: "system.overview", title: () => t("system.tab.overview"), render: () => <SubAgentOverview view={view} session={session} parent={parent} /> }];
  }
  const target: InspectorTarget = { kind: "session", ...session, ...(picked.kind === "session" && picked.tool ? { tool: picked.tool } : {}) };
  const tabs: SpaceTab[] = [
    { id: "system.overview", title: () => t("system.tab.overview"), render: () => <SessionOverview view={view} session={session} /> },
    ...inspectorTabs(view, target).map((tab) => asSpaceTab(view, tab, target)),
    { id: "system.context", title: () => t("system.tab.context"), render: () => <ContextTab view={view} session={session} /> },
    { id: "system.tasks", title: () => t("system.tab.tasks"), render: () => <TasksTab view={view} session={session} /> },
    { id: "system.frames", title: () => t("system.tab.frames"), render: () => <FramesTab host={view.host} session={session} /> },
  ];
  // A task opens on its own tab.
  if (picked.kind === "task") tabs.unshift(tabs.splice(tabs.findIndex((x) => x.id === "system.tasks"), 1)[0]!);
  return tabs;
}
