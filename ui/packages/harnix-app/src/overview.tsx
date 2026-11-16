/**
 * A project's overview (harnix-app.md §3.3): the right column of the projects space while a project
 * is open and no conversation is — the new conversation's page. It is the space's own tab
 * (`details.tabs`), there only then, so a conversation's column is its docking surface as anywhere.
 *
 * The page is the blocks of `harnix-app.project.panels`, by `order`. This package's own are
 * registered there like anyone's: what waits on a person in the project (10), its conversations
 * (20), the files its conversations changed lately (50), its settings in force with where each
 * comes from (60), and its trust (70). The flows put their tasks at 30; a schedule's block goes
 * between (40). Read-only: each block links to where a thing is changed.
 */

import { useEffect, useState, type ReactNode } from "react";
import { CHANGES_NS, errorText, parseTurnChanges, projectRelative, type McpList, type SessionEntry, type SessionSkills } from "@harnix/core";
import { Contributed, ago, t, useContributions, useT, type HostApi, type Registry } from "@harnix/react";

import { trustCategories, trustDialog, untrust } from "./dialogs.tsx";
import { InboxList, type Inbox } from "./inbox.tsx";
import "./overview.css";
import { PROJECT_PANELS, type ProjectItemContext, type ProjectPanel } from "./items.ts";
import { useApp } from "./sessions.tsx";
import { CHANGES_KIND } from "./sidebar.tsx";
import { SESSIONS, keyOf, type AppSession, type AppStore } from "./store.ts";

/** The overview's tab id in the projects space's strip. */
export const OVERVIEW = "harnix-app.overview";

/** How many rows a block lists. */
const SESSIONS_SHOWN = 5;
const CHANGES_SHOWN = 8;
/** How many of the project's latest conversations the recent changes are read from. */
const CHANGES_FROM = 5;

/** The project's conversations, newest activity first; the archived ones only while they are shown. */
export function projectSessions(sessions: AppSession[], workspaceId: string, showArchived: boolean): AppSession[] {
  return sessions
    .filter((s) => s.workspace_id === workspaceId && (!s.archived || showArchived))
    .sort((a, b) => String(b.last_active ?? "").localeCompare(String(a.last_active ?? "")));
}

/** The panels in force, by `order` (100 when absent), then as registered. */
export function orderedPanels<T extends Pick<ProjectPanel, "order">>(panels: T[]): T[] {
  return panels.map((panel, i) => ({ panel, i })).sort((a, b) => (a.panel.order ?? 100) - (b.panel.order ?? 100) || a.i - b.i).map((x) => x.panel);
}

/** The overview of the project open in the projects space. */
export function OverviewTab({ r, store }: { r: Registry; store: AppStore }) {
  const tt = useT();
  const s = useApp(store);
  const panels = orderedPanels(useContributions<ProjectPanel>(PROJECT_PANELS, r));
  const workspace = s.workspaces.find((w) => w.id === s.project);
  if (!workspace) return <div className="hx-overview empty">{tt("overview.none")}</div>;
  const context: ProjectItemContext = { root: workspace.path, workspace };
  return (
    <div className="hx-overview" data-testid="project-overview" data-project={workspace.id}>
      <div className="hx-overview-head">
        <span className="name">{workspace.title ?? basename(workspace.path)}</span>
        <span className="path" title={workspace.path}>
          {workspace.path}
        </span>
      </div>
      {panels.map((panel) => (
        <Contributed key={`${panel.owner}/${panel.id}`} registry={r} point={PROJECT_PANELS} contribution={panel}>
          <Block id={panel.id} title={panel.title}>
            <PanelBody panel={panel} context={context} />
          </Block>
        </Contributed>
      ))}
    </div>
  );
}

/** A block's body, called inside its guard: a `render` that throws costs that block, not the page. */
function PanelBody({ panel, context }: { panel: ProjectPanel; context: ProjectItemContext }) {
  return <>{panel.render(context)}</>;
}

function Block({ id, title, children }: { id: string; title: () => string; children: ReactNode }) {
  return (
    <section className="hx-overview-block" data-testid="overview-block" data-block={id}>
      <h3>{title()}</h3>
      {children}
    </section>
  );
}

/* ── this package's blocks ────────────────────────────────────────────── */

/** Register this package's blocks of the overview. */
export function registerOverview(r: Registry, host: HostApi, store: AppStore, inbox: Inbox, owner: string): void {
  const panel = (id: string, order: number, title: () => string, render: (context: ProjectItemContext) => ReactNode) =>
    r.register(PROJECT_PANELS, { id, order, title, render }, owner);
  panel("harnix-app.inbox", 10, () => t("inbox.title"), (c) => <InboxList r={r} inbox={inbox} root={c.root} limit={6} />);
  panel("harnix-app.sessions", 20, () => t("overview.sessions"), (c) => <SessionsBlock host={host} store={store} context={c} />);
  panel("harnix-app.changes", 50, () => t("overview.changes"), (c) => <ChangesBlock host={host} store={store} context={c} />);
  panel("harnix-app.settings", 60, () => t("overview.settings"), (c) => <SettingsBlock host={host} store={store} context={c} />);
  panel("harnix-app.trust", 70, () => t("overview.trust"), (c) => <TrustBlock host={host} store={store} context={c} />);
}

/** The project's latest conversations, each opening it; how many there are in all. */
function SessionsBlock({ host, store, context }: { host: HostApi; store: AppStore; context: ProjectItemContext }) {
  const tt = useT();
  const s = useApp(store);
  const sessions = projectSessions(s.sessions, context.workspace.id, s.showArchived);
  if (!sessions.length) return <div className="empty">{tt("projects.noSessions")}</div>;
  return (
    <div className="hx-overview-rows">
      {sessions.slice(0, SESSIONS_SHOWN).map((session) => (
        <button
          type="button"
          key={session.session_id}
          className="hx-overview-row"
          data-testid="overview-session"
          data-sid={session.session_id}
          onClick={() => void host.session.open({ instance: session.instance, sessionId: session.session_id })}
        >
          {s.running[keyOf(session.instance, session.session_id)] ? <span className="live" /> : null}
          <span className="name">{session.name || tt("sidebar.untitled")}</span>
          <span className="meta">{ago(session.last_active, tt)}</span>
        </button>
      ))}
      {sessions.length > SESSIONS_SHOWN ? <div className="hx-overview-note">{tt("projects.all", { count: sessions.length })}</div> : null}
    </div>
  );
}

/** A file a conversation of the project changed, as the block lists it: the latest change of each. */
export interface RecentChange {
  path: string;
  op: "create" | "modify";
  added: number;
  removed: number;
  at: string | null;
  session: { instance: string; sessionId: string; name: string | null };
  turnId: string;
}

/** The files changed, newest first, each once: its latest change. */
export function recentChanges(read: { session: RecentChange["session"]; entries: SessionEntry[] }[], limit = CHANGES_SHOWN): RecentChange[] {
  const all: RecentChange[] = [];
  for (const { session, entries } of read) {
    for (const entry of entries) {
      const turn = parseTurnChanges(entry);
      if (!turn) continue;
      for (const file of turn.files) {
        all.push({ path: file.path, op: file.op, added: file.added, removed: file.removed, at: entry.ts ?? null, session, turnId: turn.turnId });
      }
    }
  }
  all.sort((a, b) => String(b.at ?? "").localeCompare(String(a.at ?? "")));
  const seen = new Set<string>();
  return all.filter((c) => !seen.has(c.path) && seen.add(c.path)).slice(0, limit);
}

/**
 * The files the project's latest conversations changed (their `harnix.changes` entries): each
 * opens its conversation on the changes tab, at the turn. Read again when a turn of one ends.
 */
function ChangesBlock({ host, store, context }: { host: HostApi; store: AppStore; context: ProjectItemContext }) {
  const tt = useT();
  const s = useApp(store);
  const sessions = projectSessions(s.sessions, context.workspace.id, false).slice(0, CHANGES_FROM);
  const ids = sessions.map((x) => `${x.instance}/${x.session_id}`).join(",");
  const [changes, setChanges] = useState<RecentChange[] | { error: string } | null>(null);
  const [generation, setGeneration] = useState(0);
  useEffect(
    () =>
      host.events.onTurnSettled((e) => {
        const sid = (e as { session_id?: string }).session_id;
        if (sid && sessions.some((x) => x.session_id === sid)) setGeneration((g) => g + 1);
      }),
    [host, ids],
  );
  useEffect(() => {
    let live = true;
    void Promise.all(
      sessions.map(async (x) => {
        const listed = await host
          .call<{ entries?: SessionEntry[] }>("engine.session.entries", { instance: x.instance, session_id: x.session_id, space: SESSIONS, namespaces: [CHANGES_NS] })
          .catch(() => ({ entries: [] as SessionEntry[] }));
        return { session: { instance: x.instance, sessionId: x.session_id, name: x.name ?? null }, entries: listed.entries ?? [] };
      }),
    )
      .then((read) => live && setChanges(recentChanges(read)))
      .catch((e: unknown) => live && setChanges({ error: errorText(e) }));
    return () => {
      live = false;
    };
  }, [host, ids, generation]);
  if (!changes) return <div className="empty">{tt("common.loading")}</div>;
  if ("error" in changes) return <div className="empty bad">{changes.error}</div>;
  if (!changes.length) return <div className="empty">{tt("overview.changes.none")}</div>;
  const open = async (change: RecentChange) => {
    try {
      await host.session.open({ instance: change.session.instance, sessionId: change.session.sessionId });
      host.sidebar.openTab(CHANGES_KIND, { params: { turn: change.turnId } });
    } catch (e) {
      host.shell.notice(errorText(e), "error");
    }
  };
  return (
    <div className="hx-overview-rows">
      {changes.map((change) => (
        <button
          type="button"
          key={change.path}
          className="hx-overview-row"
          data-testid="overview-change"
          data-path={projectRelative(change.path, context.root)}
          title={`${change.session.name || tt("sidebar.untitled")} · ${change.path}`}
          onClick={() => void open(change)}
        >
          <span className="name mono">{projectRelative(change.path, context.root)}</span>
          {change.op === "create" ? <span className="hx-tag">{tt("changes.created")}</span> : null}
          <span className="hx-changes-counts">
            <span className="add">+{change.added}</span>
            <span className="del">−{change.removed}</span>
          </span>
          <span className="meta">{ago(change.at ?? undefined, tt)}</span>
        </button>
      ))}
    </div>
  );
}

/** A field of `settings.describe`, as far as the block reads it. */
interface DescribedField {
  key: string;
  effective?: unknown;
  source?: string | null;
  withheld?: boolean;
  tiers?: Record<string, unknown>;
}

const TIERS = ["global", "scene", "project"] as const;

/** Rules each tier writes, as "全局 2 · 当前项目 1"; nothing when none does. */
export function rulesByTier(field: DescribedField | undefined): string {
  if (!field?.tiers) return "";
  return TIERS.flatMap((tier) => {
    const rows = field.tiers?.[tier];
    return Array.isArray(rows) && rows.length ? [`${tierName(tier)} ${rows.length}`] : [];
  }).join(" · ");
}

function tierName(tier: string): string {
  return t(`overview.tier.${tier}`);
}

/** The skills a conversation of the project sees, by where they come from: "项目 3 · 内置 5". */
export function skillsByTier(skills: SessionSkills | null): string {
  if (!skills?.listed) return "";
  const counts = new Map<string, number>();
  for (const skill of skills.skills) counts.set(skill.tier, (counts.get(skill.tier) ?? 0) + 1);
  return [...counts].map(([tier, n]) => `${t(`overview.skillTier.${tier}`)} ${n}`).join(" · ");
}

/**
 * The project's settings in force and where each comes from: the permission mode and the rules
 * (`settings.describe` for the project), the MCP servers of each tier (`settings.mcp.list`), the
 * skills its latest conversation sees (`harnix.session.skills`). What a product does not offer
 * is left out. Each links to its section of the settings.
 */
function SettingsBlock({ host, store, context }: { host: HostApi; store: AppStore; context: ProjectItemContext }) {
  const tt = useT();
  const s = useApp(store);
  const latest = projectSessions(s.sessions, context.workspace.id, false)[0] ?? null;
  const [fields, setFields] = useState<DescribedField[] | null | "failed">(null);
  const [mcp, setMcp] = useState<McpList | null | "failed">(null);
  const [skills, setSkills] = useState<SessionSkills | null>(null);
  useEffect(() => {
    let live = true;
    host
      .call<{ fields?: DescribedField[] }>("settings.describe", { project_root: context.root })
      .then((d) => live && setFields(d.fields ?? []))
      .catch(() => live && setFields("failed"));
    host
      .call<McpList>("settings.mcp.list", { project_root: context.root })
      .then((d) => live && setMcp(d))
      .catch(() => live && setMcp("failed"));
    return () => {
      live = false;
    };
  }, [host, context.root]);
  useEffect(() => {
    let live = true;
    setSkills(null);
    if (latest) {
      host
        .call<SessionSkills>("harnix.session.skills", { instance: latest.instance, session_id: latest.session_id, space: SESSIONS })
        .then((d) => live && setSkills(d))
        .catch(() => undefined);
    }
    return () => {
      live = false;
    };
  }, [host, latest?.instance, latest?.session_id]);
  const field = (key: string) => (Array.isArray(fields) ? fields.find((f) => f.key === key) : undefined);
  const mode = field("permission_mode");
  const rules = field("permission_rules");
  const servers = mcp && mcp !== "failed" ? mcp.servers.filter((x) => x.enabled && !x.overridden_by) : [];
  const line = (id: string, label: string, value: ReactNode, section: string | null, extra?: ReactNode) => (
    <div className="hx-overview-setting" data-testid="overview-setting" data-setting={id}>
      <span className="label">{label}</span>
      <span className="value">
        {value}
        {extra}
      </span>
      {section ? (
        <button type="button" className="btn sm ghost" data-testid="overview-setting-open" onClick={() => host.settings.open(section)}>
          {tt("overview.settings.open")}
        </button>
      ) : null}
    </div>
  );
  const withheld = <span className="hx-tag warn">{tt("overview.settings.withheld")}</span>;
  return (
    <div className="hx-overview-settings">
      {fields === null ? <div className="empty">{tt("common.loading")}</div> : null}
      {mode
        ? line(
            "permission_mode",
            tt("overview.settings.mode"),
            `${tt(`settings.option.permission_mode.${String(mode.effective ?? "default")}`)}${mode.source ? ` · ${tierName(mode.source)}` : ""}`,
            "permissions",
            mode.withheld ? withheld : null,
          )
        : null}
      {fields !== null && fields !== "failed"
        ? line("permission_rules", tt("overview.settings.rules"), rulesByTier(rules) || tt("overview.settings.noRules"), "permissions", rules?.withheld ? withheld : null)
        : null}
      {mcp !== null && mcp !== "failed"
        ? line(
            "mcp",
            tt("overview.settings.mcp"),
            servers.length ? servers.map((x) => `${x.name} (${tierName(x.tier)})`).join(", ") : tt("overview.settings.noMcp"),
            "mcp",
          )
        : null}
      {line("skills", tt("overview.settings.skills"), latest ? skillsByTier(skills) || tt("common.loading") : tt("overview.settings.skillsNoSession"), null, skills?.skills.some((x) => x.withheld) ? withheld : null)}
    </div>
  );
}

/** Whether the project is trusted, from where, and what is held back while it is not; trusting it,
 *  or no longer, as its menu does. */
function TrustBlock({ host, store, context }: { host: HostApi; store: AppStore; context: ProjectItemContext }) {
  const tt = useT();
  const s = useApp(store);
  useEffect(() => store.ensureTrust(context.root), [store, context.root]);
  const status = s.trust[context.root];
  const name = context.workspace.title ?? basename(context.root);
  if (status === undefined) return <div className="empty">{tt("common.loading")}</div>;
  if (status === null) return <div className="empty">{tt("overview.trust.unknown")}</div>;
  return (
    <div className="hx-overview-trust" data-testid="overview-trust" data-trust={status.trust}>
      <div className="state">{tt(`overview.trust.${status.trust}`)}</div>
      {status.trust === "untrusted" && status.withheld.length ? <div className="hx-overview-note">{tt("projects.trust.banner", { categories: trustCategories(status) })}</div> : null}
      {status.trust === "untrusted" ? (
        <button type="button" className="btn sm primary" data-testid="overview-trust-action" onClick={() => void trustDialog(host, store, context.root, name)}>
          {tt("projects.trust.bannerAction")}
        </button>
      ) : status.trust === "trusted" ? (
        <button type="button" className="btn sm ghost" data-testid="overview-trust-action" onClick={() => void untrust(host, store, context.root, name)}>
          {tt("projects.trust.revoke")}
        </button>
      ) : null}
    </div>
  );
}

function basename(path: string): string {
  return path.replace(/\/+$/, "").split("/").pop() || path;
}
