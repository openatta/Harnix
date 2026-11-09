/** The system space's centre: the process at a glance, then the Agents, the flows, the applications and the diagnostics. */

import { useRef, useState } from "react";
import { useT } from "@harnix/react";

import type { App } from "../app.ts";
import { Connections } from "../dialogs.tsx";
import { useShell } from "../hooks.ts";
import { Diagnostics } from "../settings.tsx";
import { Agents } from "./agents.tsx";
import { AppsTab, appsOffered } from "./apps.tsx";
import { isActive, useAgentRows, useRunningTasks } from "./data.ts";
import { systemFlows } from "./flows.ts";
import { uptime } from "./format.ts";
import type { ConsoleTab, InstanceRow, Process } from "./types.ts";
import { useNow, useTopic, type View } from "./view.ts";

export function Console({ app, view }: { app: App; view: View }) {
  const tr = useT();
  const [tab, setShown] = useState<ConsoleTab>(view.consoleTab);
  const setTab = (id: ConsoleTab) => {
    view.consoleTab = id;
    setShown(id);
  };
  // The flows, where the backend has the flow module: the application's toolchain.
  const flows = systemFlows(app);
  const tabs: [ConsoleTab, string][] = [
    ["active", tr("system.agents.active")],
    ["all", tr("system.agents.all")],
    ...(flows ? [["flows", flows.title()] as [ConsoleTab, string]] : []),
    // The runtime applications, where this process hosts them.
    ...(appsOffered(app) ? [["apps", tr("system.apps")] as ["apps", string]] : []),
    ["diagnostics", tr("system.diagnostics")],
  ];
  return (
    <div className="system-console" data-testid="system-console">
      <Strip app={app} view={view} />
      <div className="stabs" role="tablist">
        {tabs.map(([id, label]) => (
          <button key={id} type="button" role="tab" data-tab={id} aria-selected={tab === id} className={tab === id ? "on" : ""} onClick={() => setTab(id)}>
            {label}
          </button>
        ))}
      </div>
      {tab === "diagnostics" ? (
        <div className="system-diagnostics settings-page"><Diagnostics app={app} /></div>
      ) : tab === "apps" ? (
        <AppsTab app={app} />
      ) : tab === "flows" ? (
        <div className="system-pane system-flows">{flows?.render() ?? null}</div>
      ) : (
        <Agents key={tab} app={app} view={view} scope={tab} />
      )}
    </div>
  );
}

/** The process in one row: versions, how long it has run, its memory and threads, who is connected, and the Agents. */
function Strip({ app, view }: { app: App; view: View }) {
  const tr = useT();
  const s = useShell(app);
  // Pushed (the hub computes it again every few seconds while it is watched); the uptime goes on by
  // the second here between two pushes.
  const process = useTopic<Process>(view, "harnix.process");
  const heard = useRef<{ at: number; value: Process | undefined }>({ at: Date.now(), value: undefined });
  if (heard.current.value !== process) heard.current = { at: Date.now(), value: process };
  useNow(1000);
  const instances = useTopic<{ instances: InstanceRow[] }>(view, "harnix.instances");
  const rows = useAgentRows(view, instances?.instances ?? []);
  const turns = new Set(Object.keys(s.running));
  const busy = rows.filter((row) => row.tasks).map((row) => row.key);
  const tasks = useRunningTasks(view, [...new Set([...turns, ...busy])]);
  const users = process?.connections ?? [];
  const kpi = (id: string, value: string, label: string, extra?: React.ReactNode) => (
    <div className={`kpi${extra ? " has-tip" : ""}`} data-kpi={id} title={value}>
      <div className="v">{id === "harnix" || id === "agix" ? value.split("-")[0] : value}</div>
      <div className="l">{label}</div>
      {extra}
    </div>
  );
  return (
    <div className="strip" data-testid="system-strip">
      {kpi("harnix", process?.harnix ?? "—", "Harnix")}
      {kpi("agix", process?.agix ?? "—", "Agix")}
      {kpi("uptime", process?.uptime_secs != null ? uptime(process.uptime_secs + Math.floor((Date.now() - heard.current.at) / 1000)) : "—", tr("system.uptime"))}
      {kpi("memory", process?.peak_rss_bytes ? `${Math.round(process.peak_rss_bytes / 1048576)} MB` : "—", tr("system.peakMemory"))}
      {kpi("threads", process?.threads != null ? String(process.threads) : "—", tr("system.threads"))}
      {kpi(
        "users",
        String(users.length),
        tr("system.users"),
        <div className="tip2">
          <div className="tip2-h">{tr("process.users")}</div>
          <Connections connections={users} />
        </div>,
      )}
      {kpi("instances", String(instances?.instances.length ?? 0), tr("system.instances"))}
      {kpi("agents", `${rows.filter((row) => isActive(row, turns)).length}/${rows.length}`, tr("system.kpi.agents"))}
      {kpi("sessions", String(turns.size), tr("system.runningSessions"))}
      {kpi("tasks", String(tasks), tr("system.runningTasks"))}
      {kpi("waiting", String(rows.filter((row) => row.waiting).length), tr("system.kpi.waiting"))}
    </div>
  );
}
