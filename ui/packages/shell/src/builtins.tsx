/**
 * The shell's own contributions, registered through the same points a package's go through, so
 * there is no private path for them to take.
 *
 * Kinds the conversation trunk draws itself (messages, thinking, asks, sub-agents, images) need no
 * registration; what is registered here is what the shell adds. The settings panel's own sections
 * are its frame's (`settings.tsx`), not registrations.
 */

import type { Block, BlockOf } from "@harnix/core/state";
import { Disclosure, STATUS_GLYPH, ToolDetails, t, toolArgs, toolDisplayName, TOOL_NAMES, useToolState, type ActionTarget, type DetailItem, Glyph } from "@harnix/react";

import type { App } from "./app.ts";
import { Labelled, RequestPanel, RequestRow, type RequestBlock } from "./envelope.tsx";
import { startInline } from "./inline/index.tsx";
import { ActionArg, ActionButtons } from "./actions.tsx";
import { GUIDE_KIND } from "./sidebar/controller.ts";
import { GuideTab } from "./sidebar/Guide.tsx";
import { registerShellShortcuts } from "./shortcuts.tsx";

const OWNER = "builtin";

/**
 * The shell's own registrations: the "open in details" door on a tool row, the request envelope
 * and its pane, the engine's commands, the right column's guide page, and the settings sections every product has. No space:
 * every space is a UI package's or the application's. Then what extension packages declare
 * inline, drawn by the shell.
 */
export function registerBuiltins(app: App): void {
  const r = app.registry;
  r.register("tool.row", { id: "builtin.tool", match: () => true, render: (block) => <ToolRow app={app} block={block} /> }, OWNER);
  registerShellShortcuts(app);

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

  // The right column's default page (harnix-ext.md §22.7): a builtin, so an extension's `guide` type can take its place.
  r.register(
    "sidebar.tab",
    {
      id: "builtin.guide",
      kind: GUIDE_KIND,
      priority: "builtin",
      title: () => t("sidebar.guide"),
      render: () => <GuideTab app={app} />,
    },
    OWNER,
  );

  // What extension packages declare inline, drawn by the shell: registered as they start.
  startInline(app);
}

/* ── tool rows ────────────────────────────────────────────────────────── */

function ToolRow({ app, block }: { app: App; block: BlockOf<"tool"> }) {
  const { state, tail } = useToolState(block);
  const current = app.snapshot.current;
  const target: ActionTarget | null =
    current && block.id
      ? { kind: "tool", session: { instance: current.instance, sessionId: current.sessionId }, space: app.snapshot.space, toolUseId: block.id, tool: block.name, input: block.input }
      : null;
  return (
    <Disclosure
      testId="conv-tool"
      id={block.id}
      state={state}
      summary={
        <>
          <span className={`hx-status hx-status-${block.status}`} aria-hidden>
            {STATUS_GLYPH[block.status]}
          </span>
          <span className="hx-tool-name" data-testid="conv-tool-name" title={block.name}>
            {toolDisplayName(block.name, TOOL_NAMES)}
          </span>
          {target ? (
            <ActionArg app={app} target={target} className="hx-tool-arg" testId="conv-tool-args">
              {toolArgs(block.input)}
            </ActionArg>
          ) : (
            <span className="hx-tool-arg" data-testid="conv-tool-args">
              {toolArgs(block.input)}
            </span>
          )}
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
        {target ? <ActionButtons app={app} place="tool" target={target} /> : null}
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
