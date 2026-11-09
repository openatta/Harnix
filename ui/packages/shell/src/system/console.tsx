/** The system space's centre: the process at a glance, then the Agents, the flows, the applications and the diagnostics. */

import { useRef, useState, type ReactNode } from "react";
import { useT } from "@harnix/react";

import type { App } from "../app.ts";
import { Connections } from "../dialogs.tsx";
import { clock } from "../format.ts";
import { useShell } from "../hooks.ts";
import { useRovingTabIndex } from "../roving.ts";
import { Diagnostics } from "../settings.tsx";
import { Agents } from "./agents.tsx";
import { AppsTab, appsOffered } from "./apps.tsx";
import { isActive, useAgentRows, useRunningTasks } from "./data.ts";
import { systemFlows } from "./flows.ts";
import type { ConsoleTab, InstanceRow, Process } from "./types.ts";
import { useTick, useTopic, type View } from "./view.ts";

/**
 * The console: the strip, then its tabs. The tab strip is one Tab stop, its arrows moving between
 * the tabs. A tab with nothing to pick on the right (the flows, the applications, the diagnostics)
 * lets go of what was picked there, and the right column says what that tab is.
 */
export function Console({ app, view }: { app: App; view: View }) {
  const tr = useT();
  const [tab, setShown] = useState<ConsoleTab>(view.consoleTab);
  const setTab = (id: ConsoleTab) => {
    view.showTab(id);
    setShown(id);
  };
  const strip = useRef<HTMLDivElement>(null);
  const roving = useRovingTabIndex(strip, { items: '[role="tab"]', orientation: "horizontal", activate: true });
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
      <div className="stabs" role="tablist" aria-label={tr("space.system")} ref={strip} onKeyDown={roving.onKeyDown} onFocus={roving.onFocus}>
        {tabs.map(([id, label]) => (
          <button
            key={id}
            type="button"
            role="tab"
            id={`system-tab-${id}`}
            aria-controls="system-tabpanel"
            data-tab={id}
            aria-selected={tab === id}
            className={tab === id ? "on" : ""}
            onClick={() => setTab(id)}
          >
            {label}
          </button>
        ))}
      </div>
      <div role="tabpanel" id="system-tabpanel" aria-labelledby={`system-tab-${tab}`} className="system-tabpanel">
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
    </div>
  );
}

/** The process in one row: versions, how long it has run, its memory and threads, who is connected, and the Agents. */
function Strip({ app, view }: { app: App; view: View }) {
  const tr = useT();
  const s = useShell(app);
  // Pushed (the hub computes it again every few seconds while it is watched); the uptime goes on by
  // the second between two pushes, in its own cell.
  const process = useTopic<Process>(view, "harnix.process");
  const heard = useRef<{ at: number; value: Process | undefined }>({ at: Date.now(), value: undefined });
  if (heard.current.value !== process) heard.current = { at: Date.now(), value: process };
  const instances = useTopic<{ instances: InstanceRow[] }>(view, "harnix.instances");
  const rows = useAgentRows(view, instances?.instances ?? []);
  const turns = new Set(Object.keys(s.running));
  const busy = rows.filter((row) => row.tasks).map((row) => row.key);
  const tasks = useRunningTasks(view, [...new Set([...turns, ...busy])]);
  const users = process?.connections ?? [];
  const kpi = (id: string, value: string | ReactNode, label: string, extra?: ReactNode) => (
    <div className={`kpi${extra ? " has-tip" : ""}`} data-kpi={id} title={typeof value === "string" ? value : undefined}>
      <div className="v">{typeof value === "string" && (id === "harnix" || id === "agix") ? value.split("-")[0] : value}</div>
      <div className="l">{label}</div>
      {extra}
    </div>
  );
  return (
    <div className="strip" data-testid="system-strip">
      {kpi("harnix", process?.harnix ?? "—", "Harnix")}
      {kpi("agix", process?.agix ?? "—", "Agix")}
      {kpi("uptime", process?.uptime_secs != null ? <Uptime secs={process.uptime_secs} heard={heard.current.at} /> : "—", tr("system.uptime"))}
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

/** How long the process has run: what it last said, counted on by the second. */
function Uptime({ secs, heard }: { secs: number; heard: number }) {
  useTick(1000);
  return <>{clock(secs + Math.floor((Date.now() - heard) / 1000))}</>;
}

const ABOUT: Record<ConsoleTab, string> = {
  active: "system.about.agents",
  all: "system.about.agents",
  flows: "system.about.flows",
  apps: "system.about.apps",
  diagnostics: "system.about.diagnostics",
};

/** The right column while nothing is picked: what the console's tab in front is, and what to do there. */
export function ConsoleAbout({ app, view }: { app: App; view: View }) {
  const tr = useT();
  const tab = view.consoleTab;
  const title = tab === "active" || tab === "all" ? tr("system.agents") : tab === "flows" ? (systemFlows(app)?.title() ?? tr("system.flows")) : tr(tab === "apps" ? "system.apps" : "system.diagnostics");
  return (
    <div className="system-pane" data-testid="system-about" data-tab={tab}>
      <div className="label">{title}</div>
      <div className="note">{tr(ABOUT[tab])}</div>
    </div>
  );
}
