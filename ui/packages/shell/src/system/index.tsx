/**
 * The system space (design §9): what this process runs, and the hands to act on it.
 *
 * Its three columns: on the left, the instances; in the centre, the console — the process at a
 * glance, then the Agents (the active ones, or all of them: each main Agent on a row with its
 * sub-agents, tasks, tokens and model time, opened into its sub-agents and tasks), the flows
 * where the backend has the flow module (a flow clicked there is a space of its own shown inside
 * this one: `flows.ts`), the applications, and the diagnostics; on the right, whatever was
 * picked there:
 * an instance's facts and its restart, or a session's facts and operations, every model call with
 * what went in and what came out, the context the next call will carry, its tasks, and its frames
 * as they arrive. There is no conversation here: the system space looks at sessions, it does not
 * hold one.
 *
 * Registered only when the backend offers the system methods (`[shell] spaces` keeps `system`),
 * and built on the public calls alone: `harnix.system.*` for what the engine keeps behind the
 * system module, `session.*` and `harnix.instances.*` claiming the system space for the rest —
 * every one of them audited. What Agix does not offer has no tab here (§9.3).
 */

import { t, type SpaceTab } from "@harnix/react";

import type { App, SessionKey } from "../app.ts";
import { OWNER, SYSTEM } from "./constants.ts";
import { Console } from "./console.tsx";
import { InstanceList } from "./instances.tsx";
import { CallsTab } from "./pane/calls.tsx";
import { ContextTab } from "./pane/context.tsx";
import { FramesTab } from "./pane/frames.tsx";
import { InstanceOverview } from "./pane/instance.tsx";
import { SessionOverview } from "./pane/session.tsx";
import { SpaceSessions } from "./pane/space.tsx";
import { SubAgentOverview } from "./pane/subagent.tsx";
import { TasksTab } from "./pane/tasks.tsx";
import { TurnsTab } from "./pane/turns.tsx";
import { View } from "./view.ts";

export { SYSTEM };
export type { Selection } from "./view.ts";

/** The system space's view, once it is registered: what `openInSystem` drives. */
let registered: View | null = null;

/** Whether this product keeps the system space, so a way into it may be offered. */
export function systemOffered(): boolean {
  return registered !== null;
}

/**
 * Into the system space from a conversation. With `tool`: that session's calls, the call that asked
 * for the tool picked out. Without: the sessions of the space the person came from, theirs first —
 * an application has many, and the one in front is only one of them.
 */
export async function openInSystem(app: App, target: { instance: string; sessionId: string; space: string; tool?: string }): Promise<void> {
  const view = registered;
  if (!view) return;
  await app.switchSpace(SYSTEM);
  const session = { instance: target.instance, sessionId: target.sessionId };
  if (target.tool) {
    view.focusTool(target.tool);
    view.select({ kind: "session", ...session });
    app.selectTab("system.calls", true);
  } else {
    view.select({ kind: "space", space: target.space, current: session });
  }
}

export function registerSystem(app: App): void {
  if (!app.snapshot.reachable.includes("harnix.system.context")) return;
  const view = new View(app);
  registered = view;
  app.registry.register(
    "space",
    {
      id: SYSTEM,
      icon: "cpu",
      order: 90,
      title: () => t("space.system"),
      subtitle: () => t("space.system.subtitle"),
      center: "hidden",
      detailsWidth: 0.45,
      overview: () => <Console app={app} view={view} />,
      sidebar: () => <InstanceList app={app} view={view} />,
      tabs: () => tabsFor(app, view),
    },
    OWNER,
  );
}

/** The right column's tabs for what is selected: only the ones that apply. */
function tabsFor(app: App, view: View): SpaceTab[] {
  const picked = view.selected();
  if (!picked) return [];
  if (picked.kind === "space") {
    return [{ id: "system.overview", title: () => t("system.tab.spaceSessions"), render: () => <SpaceSessions app={app} view={view} space={picked.space} current={picked.current ?? null} /> }];
  }
  if (picked.kind === "instance") {
    return [{ id: "system.overview", title: () => t("system.tab.overview"), render: () => <InstanceOverview app={app} view={view} instance={picked.instance} /> }];
  }
  const session: SessionKey = { instance: picked.instance, sessionId: picked.sessionId };
  // A sub-agent's calls are in its parent's recording and its runtime state in its parent's tasks.
  if (picked.kind === "session" && picked.parent) {
    const parent = picked.parent;
    return [{ id: "system.overview", title: () => t("system.tab.overview"), render: () => <SubAgentOverview app={app} view={view} session={session} parent={parent} /> }];
  }
  const tabs: SpaceTab[] = [
    { id: "system.overview", title: () => t("system.tab.overview"), render: () => <SessionOverview app={app} view={view} session={session} /> },
    { id: "system.calls", title: () => t("system.tab.calls"), render: () => <CallsTab app={app} view={view} session={session} /> },
    { id: "system.turns", title: () => t("system.tab.turns"), render: () => <TurnsTab app={app} session={session} /> },
    { id: "system.context", title: () => t("system.tab.context"), render: () => <ContextTab app={app} view={view} session={session} /> },
    { id: "system.tasks", title: () => t("system.tab.tasks"), render: () => <TasksTab app={app} view={view} session={session} /> },
    { id: "system.frames", title: () => t("system.tab.frames"), render: () => <FramesTab app={app} session={session} /> },
  ];
  // A task opens on its own tab.
  if (picked.kind === "task") tabs.unshift(tabs.splice(tabs.findIndex((x) => x.id === "system.tasks"), 1)[0]!);
  return tabs;
}
