/**
 * The product's own contributions, registered through the same points a package's go through,
 * so there is no private path for the built-in ones to take.
 *
 * Kinds the conversation trunk draws itself (messages, thinking, asks, sub-agents, images) need no
 * registration; what is registered here is what the shell adds: the "open in details" door on a
 * tool row, the request envelope, the workspace grouping, the engine's commands and the settings
 * pages.
 */

import type { Block, BlockOf } from "@harnix/core/state";
import { Disclosure, STATUS_GLYPH, ToolDetails, t, toolArgs, toolDisplayName, TOOL_NAMES, type DetailItem, type SidebarGroup } from "@harnix/react";

import type { App, ShellSession, Workspace } from "./app.ts";
import { Labelled, RequestPanel, RequestRow, type RequestBlock } from "./envelope.tsx";
import { Glyph } from "./glyph.tsx";
import { RecentSessions } from "./home.tsx";
import { builtinSections } from "./settings.tsx";

const OWNER = "builtin";

export function registerBuiltins(app: App): void {
  const r = app.registry;

  r.register(
    "space",
    { id: "home", icon: "home", order: 0, title: () => t("space.home"), subtitle: () => t("space.home.subtitle"), center: "minimized", placeholder: () => t("home.ask") },
    OWNER,
  );
  r.register(
    "space",
    { id: "sessions", icon: "chat", order: 10, title: () => t("space.sessions"), subtitle: () => t("space.sessions.subtitle"), center: "chat" },
    OWNER,
  );
  r.register("home.card", { id: "builtin.recent", order: 0, title: () => t("home.recent.title"), render: () => <RecentSessions app={app} /> }, OWNER);

  r.register("tool.row", { id: "builtin.tool", match: () => true, render: (block) => <ToolRow app={app} block={block} /> }, OWNER);

  r.register(
    "flow.block",
    {
      id: "builtin.request",
      match: (block: Block) => block.kind === "custom" && block.type === "request",
      render: (block: Block) => <RequestRow app={app} block={(block as BlockOf<"custom">).data as RequestBlock} />,
    },
    OWNER,
  );

  r.register(
    "details.panel",
    {
      id: "builtin.tool",
      match: (item: DetailItem) => item.kind === "tool",
      title: (item: DetailItem) => item.name ?? t("details.title"),
      render: (item: DetailItem) => <ToolPanel block={item as unknown as BlockOf<"tool">} />,
    },
    OWNER,
  );
  r.register(
    "details.panel",
    {
      id: "builtin.request",
      match: (item: DetailItem) => item.kind === "request",
      title: () => t("request.title"),
      render: (item: DetailItem) => <RequestPanel block={item as unknown as RequestBlock} />,
    },
    OWNER,
  );

  r.register("sidebar.group", { id: "builtin.workspace", label: () => t("sidebar.byWorkspace"), group: (sessions) => byWorkspace(app, sessions as ShellSession[]) }, OWNER);

  r.register(
    "command",
    {
      id: "builtin.commands",
      complete(query: string) {
        const commands = (app.snapshot.hello?.commands ?? []) as { name?: string; description?: string; source?: string }[];
        const q = query.toLowerCase();
        return commands
          .filter((c) => typeof c.name === "string" && c.name.toLowerCase().startsWith(q))
          .map((c) => ({ replacement: `/${c.name} `, label: `/${c.name}`, description: c.description, source: c.source }));
      },
    },
    OWNER,
  );

  for (const section of builtinSections(app)) r.register("settings.section", section, OWNER);
}

/* ── tool rows ────────────────────────────────────────────────────────── */

function ToolRow({ app, block }: { app: App; block: BlockOf<"tool"> }) {
  const tail =
    block.status === "running" ? t("flow.toolRunning") : block.status === "error" ? t("flow.toolFailed") : block.status === "interrupted" ? t("flow.interrupted") : undefined;
  return (
    <Disclosure
      testId="conv-tool"
      id={block.id}
      state={block.status}
      summary={
        <>
          <span className={`hx-status hx-status-${block.status}`} aria-hidden>
            {STATUS_GLYPH[block.status]}
          </span>
          <span className="hx-tool-name" data-testid="conv-tool-name" title={block.name}>
            {toolDisplayName(block.name, TOOL_NAMES)}
          </span>
          <span className="hx-tool-arg">{toolArgs(block.input)}</span>
        </>
      }
      tail={tail}
    >
      <ToolDetails block={block} />
      <div className="actions">
        <button
          type="button"
          className="btn sm outline"
          data-testid="open-in-details"
          onClick={() => app.showDetail({ ...block, kind: "tool", name: block.name })}
        >
          {t("flow.openInDetails")}
        </button>
      </div>
    </Disclosure>
  );
}

/* Unclipped: the pane is for the reader who needs all of it. */
function ToolPanel({ block }: { block: BlockOf<"tool"> }) {
  return (
    <div data-testid="details-tool">
      <div className="panel-head">
        <Glyph name="terminal" className="lead" />
        <span>{block.name || t("flow.tool")}</span>
      </div>
      {block.id ? <Labelled label={t("details.callId")}>{block.id}</Labelled> : null}
      {block.input !== undefined ? (
        <Labelled label={t("flow.input")}>{typeof block.input === "string" ? block.input : JSON.stringify(block.input, null, 2)}</Labelled>
      ) : null}
      {block.result != null ? <Labelled label={block.error ? t("flow.error") : t("flow.result")}>{block.result}</Labelled> : null}
    </div>
  );
}

/* ── grouping ─────────────────────────────────────────────────────────── */

/* Registered workspaces first, in their stored order; then sessions with no project, then those
 * Harnix has never opened — `session.list` carries no project root for those, so they get a group
 * of their own rather than a guess, and move out the moment they are opened. */
function byWorkspace(app: App, sessions: ShellSession[]): SidebarGroup[] {
  const workspaces: Workspace[] = app.snapshot.workspaces;
  const groups = workspaces.map((workspace) => ({
    key: workspace.id,
    workspace,
    title: workspace.title ?? workspace.path,
    hint: workspace.path,
    rows: [] as ShellSession[],
  }));
  const byId = new Map(groups.map((g) => [g.key, g]));
  const loose = { key: "loose", title: t("sidebar.groupLoose"), rows: [] as ShellSession[] };
  const unknown = { key: "unknown", title: t("sidebar.groupUnknown"), rows: [] as ShellSession[] };
  for (const session of sessions) {
    const group = session.workspace_id ? byId.get(session.workspace_id) : undefined;
    if (group) group.rows.push(session);
    else if (session.scene) loose.rows.push(session);
    else unknown.rows.push(session);
  }
  const sort = (rows: ShellSession[], manual?: string[]) => {
    const order = manual?.length ? manual : null;
    rows.sort((a, b) => {
      if (order) {
        const ia = order.indexOf(a.session_id);
        const ib = order.indexOf(b.session_id);
        if (ia !== -1 || ib !== -1) return (ia === -1 ? 1e9 : ia) - (ib === -1 ? 1e9 : ib);
      }
      return String(b.last_active ?? "").localeCompare(String(a.last_active ?? ""));
    });
  };
  for (const group of groups) sort(group.rows, group.workspace.session_order);
  sort(loose.rows);
  sort(unknown.rows);
  return [...groups, loose, unknown].filter((g) => g.rows.length);
}
