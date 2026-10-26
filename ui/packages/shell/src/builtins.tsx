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

import type { App, ShellSession } from "./app.ts";
import { Labelled, RequestPanel, RequestRow, type RequestBlock } from "./envelope.tsx";
import { Glyph } from "./glyph.tsx";
import { ProjectsSidebar } from "./projects.tsx";
import { SessionsSidebar } from "./sessions.tsx";
import { builtinSections } from "./settings.tsx";
import { registerSystem } from "./system.tsx";

const OWNER = "builtin";

/**
 * The three built-in spaces — conversations by time, conversations by project, the system — are
 * examples an application keeps or drops (`[shell] spaces`). Whatever it drops is not registered at
 * all, so the frame is the fixed chrome plus whichever spaces are left.
 */
export function registerBuiltins(app: App): void {
  const r = app.registry;
  const keeps = (space: string) => app.snapshot.hello?.shell?.spaces?.includes(space) ?? true;

  if (keeps("sessions")) {
    r.register(
      "space",
      {
        id: "sessions",
        icon: "chat",
        order: 10,
        title: () => t("space.sessions"),
        subtitle: () => t("space.sessions.subtitle"),
        center: "chat",
        sidebar: () => <SessionsSidebar app={app} />,
      },
      OWNER,
    );
  }
  if (keeps("projects")) {
    r.register(
      "space",
      {
        id: "projects",
        icon: "folder",
        order: 20,
        title: () => t("space.projects"),
        subtitle: () => t("space.projects.subtitle"),
        center: "chat",
        sessions: "sessions",
        sidebar: () => <ProjectsSidebar app={app} />,
      },
      OWNER,
    );
  }
  if (keeps("system")) registerSystem(app);

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

  r.register("sidebar.group", { id: "builtin.time", label: () => t("sidebar.byTime"), group: (sessions) => byTime(sessions as ShellSession[]) }, OWNER);

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

/* The session space lists everything by when it was last active: today, the last seven days,
 * earlier. Projects have a space of their own; an application that wants another grouping
 * registers its own `sidebar.group`, and the last registration wins. */
function byTime(sessions: ShellSession[]): SidebarGroup[] {
  const now = Date.now();
  const startOfToday = new Date(new Date().toDateString()).getTime();
  const groups = [
    { key: "today", title: t("time.today"), rows: [] as ShellSession[] },
    { key: "week", title: t("time.week"), rows: [] as ShellSession[] },
    { key: "earlier", title: t("time.earlier"), rows: [] as ShellSession[] },
  ];
  const sorted = [...sessions].sort((a, b) => String(b.last_active ?? "").localeCompare(String(a.last_active ?? "")));
  for (const session of sorted) {
    const at = Date.parse(session.last_active ?? "");
    const group = Number.isNaN(at) ? groups[2]! : at >= startOfToday ? groups[0]! : now - at < 7 * 86_400_000 ? groups[1]! : groups[2]!;
    group.rows.push(session);
  }
  return groups.filter((g) => g.rows.length);
}
