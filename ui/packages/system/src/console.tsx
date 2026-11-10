/** The system space's centre: the process at a glance, then the Agents and the tabs other packages put here. */

import { useRef, useState, type ReactNode } from "react";
import { Contributed, clock, since, useContributions, useRoving, useT, type Registry } from "@harnix/react";

import { CONSOLE_TAB } from "./constants.ts";
import { Agents } from "./agents.tsx";
import { isActive, useAgentRows, useRunningTasks } from "./data.ts";
import type { Connection, ConsoleTab, ConsoleTabId, InstanceRow, Process } from "./types.ts";
import { useRunning, useTick, useTopic, type View } from "./view.ts";

/** The tabs other packages put on the console (`system.console.tab`), by `order`. */
export function consoleTabs(registry: Registry): (ConsoleTab & { owner: string })[] {
  return [...registry.list<ConsoleTab>(CONSOLE_TAB)].sort((a, b) => (a.order ?? 100) - (b.order ?? 100));
}

/** A contributed tab's title; one that throws is shown by its id. */
function titleOf(tab: ConsoleTab): string {
  try {
    return tab.title();
  } catch {
    return tab.id;
  }
}

/**
 * The console: the strip, then its tabs — the Agents (active, all), then whatever other packages
 * put here (the flows, the extensions). The tab strip is one Tab stop, its arrows moving between
 * the tabs. A contributed tab has nothing to pick on the right: it lets go of what was picked
 * there, and the right column says what that tab is.
 */
export function Console({ view }: { view: View }) {
  const tr = useT();
  const contributed = [...useContributions<ConsoleTab>(CONSOLE_TAB, view.registry)].sort((a, b) => (a.order ?? 100) - (b.order ?? 100));
  const [shown, setShown] = useState<ConsoleTabId>(view.consoleTab);
  const setTab = (id: ConsoleTabId) => {
    view.showTab(id);
    setShown(id);
  };
  const strip = useRef<HTMLDivElement>(null);
  const roving = useRoving(strip, { items: '[role="tab"]', orientation: "horizontal", activate: true });
  // A contributed tab that went away (its extension stopped) leaves the Agents in front.
  const extra = contributed.find((c) => c.id === shown);
  const tab: ConsoleTabId = shown === "active" || shown === "all" || extra ? shown : "active";
  const tabs: [ConsoleTabId, string][] = [["active", tr("system.agents.active")], ["all", tr("system.agents.all")], ...contributed.map((c) => [c.id, titleOf(c)] as [string, string])];
  return (
    <div className="system-console" data-testid="system-console">
      <Strip view={view} />
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
        {extra ? (
          <div className="system-pane system-contributed" data-tab={extra.id}>
            <Contributed registry={view.registry} point={CONSOLE_TAB} contribution={extra}>
              <Drawn render={() => extra.render()} />
            </Contributed>
          </div>
        ) : (
          <Agents key={tab} view={view} scope={tab === "all" ? "all" : "active"} />
        )}
      </div>
    </div>
  );
}

/** A contribution's `render()`, called while drawing inside its guard: one that throws loses only itself. */
export function Drawn({ render }: { render: () => ReactNode }) {
  return <>{render()}</>;
}

/** What kind of client a user agent is, roughly: a desktop shell, a phone, a browser. */
export function clientKind(agent: string | undefined): "desktop" | "mobile" | "browser" | "script" {
  const ua = agent ?? "";
  if (!ua) return "script";
  if (/Tauri|Electron|harnix-desktop/i.test(ua)) return "desktop";
  if (/Mobile|Android|iPhone|iPad/i.test(ua)) return "mobile";
  if (/Mozilla|WebKit|Chrome|Safari|Firefox/i.test(ua)) return "browser";
  return "script";
}

/** The users connected right now: device, kind, how, since when. */
export function Connections({ connections }: { connections: Connection[] }) {
  const t = useT();
  if (!connections.length) return <div className="hint">{t("process.noConnections")}</div>;
  return (
    <div className="connections" data-testid="connections">
      {connections.map((c) => (
        <div key={c.client} className="conn-row" data-device={c.device}>
          <span className="dev">{c.device}</span>
          <span className="kind">{t(`process.kind.${clientKind(c.user_agent)}`)}</span>
          <span className="topo">{c.topology}</span>
          <span className="since">{c.since_ms ? since(c.since_ms) : ""}</span>
        </div>
      ))}
    </div>
  );
}

/** The process in one row: versions, how long it has run, its memory and threads, who is connected, and the Agents. */
function Strip({ view }: { view: View }) {
  const tr = useT();
  // Pushed (the hub computes it again every few seconds while it is watched); the uptime goes on by
  // the second between two pushes, in its own cell.
  const process = useTopic<Process>(view, "system.process");
  const heard = useRef<{ at: number; value: Process | undefined }>({ at: Date.now(), value: undefined });
  if (heard.current.value !== process) heard.current = { at: Date.now(), value: process };
  const instances = useTopic<{ instances: InstanceRow[] }>(view, "harnix.instances");
  const rows = useAgentRows(view, instances?.instances ?? []);
  const turns = useRunning(view);
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

/** The right column while nothing is picked: what the console's tab in front is, and what to do there. */
export function ConsoleAbout({ view }: { view: View }) {
  const tr = useT();
  const tab = view.consoleTab;
  const extra = tab === "active" || tab === "all" ? undefined : consoleTabs(view.registry).find((c) => c.id === tab);
  let about = "";
  try {
    about = extra?.about?.() ?? "";
  } catch {
    // A tab whose words fail says only its title.
  }
  return (
    <div className="system-pane" data-testid="system-about" data-tab={extra ? tab : tab === "all" ? "all" : "active"}>
      <div className="label">{extra ? titleOf(extra) : tr("system.agents")}</div>
      <div className="note">{extra ? about : tr("system.about.agents")}</div>
    </div>
  );
}
