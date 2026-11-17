/**
 * `registerFlowSpace()`: the flow module's components put together as one space (xops-02 §7), the
 * place flows are managed — listed at the bottom of the left column, as it is about the
 * application's toolchain and not one of its surfaces. The shell registers it itself when the
 * backend has the flow module; an application that builds its own shell may call it too.
 *
 *   left    the flows, the selected one's runs and its own conversation, the general
 *           conversation; the way back while a task was entered from elsewhere    (`FlowSidebar`)
 *   centre  the conversation of what is selected (xops-03 §6): a flow's own (`flow-author`, whose
 *           tools write and run it), a run's main session, or the general one; the line over the
 *           composer names its flow                                          (`composerNotice`)
 *   right   flows · runs · reference: the lists, filtered and sorted, never edited here  (`tabs`)
 *   beside  the selected flow's or run's details, between the centre and the right column
 *           (`secondary`, xops-04 §7); closing it keeps the selection and only hides the panel
 *
 * An empty conversation says what it is for (`emptyConversation`).
 *
 * See `FlowSpaceState`. Also the `flow.block` that draws a session's proposal, and the `flowui`
 * strings.
 */

import { t as translate, type HostSessionRef, type HostApi, type Registry, type SpaceContribution } from "@harnix/react";

import { actorLabel, proposalHandledOf, proposalOf, type BlockShape } from "./model";
import { ProposalCard } from "./ProposalCard";
import { FlowSidebar } from "./sidebar";
import { FlowSpaceState } from "./state";
import { registerFlowStrings, STRINGS_OWNER } from "./strings";
import { EmptyConversation, FlowSecondary, FlowsTab, FocusNotice, ReferenceTab, RunsTab, type FlowTabs } from "./tabs";
import { FlowToolRow, type ToolBlock } from "./ToolTarget";
import { toolTarget } from "./model";

export interface FlowSpaceOptions {
  /** The space's id — the backend space whose conversations run `flow-author`; "flow" by default. */
  space?: string;
  /** The id the application registered the flow extension under; "flow" by default (`flow.*`). */
  module?: string;
  /** The space's title; the `flowui.space` string by default. */
  title?: string;
  icon?: string;
  order?: number;
  group?: string;
  /** The owner the contributions are registered as; "flowui" by default. */
  owner?: string;
  /** Given: the middle follows the selection from now on, not only once the space is first drawn. */
  host?: HostApi;
}

export interface FlowSpace {
  state: FlowSpaceState;
  space: string;
  module: string;
  /** The right-column tabs' ids. */
  tabs: FlowTabs;
}

export function registerFlowSpace(registry: Registry, options: FlowSpaceOptions = {}): FlowSpace {
  const space = options.space ?? "flow";
  const module = options.module ?? "flow";
  const owner = options.owner ?? STRINGS_OWNER;
  const tabs: FlowTabs = { flows: `${space}.flows`, runs: `${space}.runs`, reference: `${space}.reference` };
  const state = new FlowSpaceState(space, module, tabs, { registry });
  if (options.host) state.attach(options.host);

  registerFlowStrings(registry);

  const parts = conversationParts(state, tabs, options);
  registry.register(
    "space",
    {
      ...parts,
      sidebar: { render: () => <FlowSidebar state={state} /> },
      details: {
        ...parts.details,
        initial: "open",
        tabs: [
          { id: tabs.flows, title: () => translate("flowui.tab.flows"), render: () => <FlowsTab state={state} tabs={tabs} /> },
          { id: tabs.runs, title: () => translate("flowui.tab.runs"), render: () => <RunsTab state={state} tabs={tabs} /> },
          { id: tabs.reference, title: () => translate("flowui.tab.reference"), render: () => <ReferenceTab state={state} />, align: "end" },
        ],
      },
    },
    owner,
  );
  registerBlocks(registry, space, module, owner);
  return { state, space, module, tabs };
}

/** What the space is: a conversation, the line over its composer, what an empty one is for, and the
 *  selection's details beside it. */
function conversationParts(state: FlowSpaceState, tabs: FlowTabs, options: FlowSpaceOptions): SpaceContribution {
  return {
    id: state.space,
    icon: options.icon ?? "fork",
    order: options.order ?? 90,
    // The toolchain, not one of the product's surfaces: at the bottom of the left column.
    place: "bottom",
    ...(options.group !== undefined ? { group: options.group } : {}),
    title: () => options.title ?? translate("flowui.space"),
    conversation: {
      subtitle: () => translate("flowui.space.subtitle"),
      placeholder: () => translate("flowui.composer.placeholder"),
      notice: () => {
        const flow = state.get().flow;
        return flow ? translate("flowui.focus.next", { flow }) : translate("flowui.focus.none");
      },
      composerNotice: (session: HostSessionRef) => <FocusNotice state={state} session={session} />,
    },
    details: { width: 400 },
    center: {
      mode: "chat",
      emptyConversation: (session: HostSessionRef) => <EmptyConversation state={state} session={session} />,
      secondary: () => {
        const detail = state.detail();
        if (!detail) return null;
        return {
          key: detail.kind === "flow" ? `flow:${detail.flow}` : `run:${detail.run}`,
          title: () => translate(detail.kind === "flow" ? "flowui.secondary.flow" : "flowui.secondary.run", { name: detail.kind === "flow" ? detail.flow : detail.run }),
          render: () => <FlowSecondary state={state} tabs={tabs} />,
          onClose: () => state.closeDetail(),
          width: 560,
        };
      },
    },
  };
}

/** The conversation's own rows and blocks, under `space`'s ids. */
function registerBlocks(registry: Registry, space: string, module: string, owner: string): void {
  // A call of the conversation's tools that changes something, named in its row by what it acts on.
  registry.register(
    "tool.row",
    {
      id: `${space}.author-tool`,
      match: (block: ToolBlock) => toolTarget(block.name, block.input) !== null,
      render: (block: ToolBlock) => <FlowToolRow block={block} />,
    },
    owner,
  );

  // The record of an accepted proposal: one quiet line where it was accepted.
  registry.register(
    "flow.block",
    {
      id: `${space}.proposal-handled`,
      match: (block: BlockShape) => proposalHandledOf(block, module) !== null,
      render: (block: BlockShape) => {
        const h = proposalHandledOf(block, module)!;
        return (
          <div className="hx-blk hx-flow-faint" data-testid="flow-proposal-handled">
            {translate("flowui.proposal.handled", { action: translate("flowui.proposal.action." + h.action), by: actorLabel(h.by) })}
          </div>
        );
      },
    },
    owner,
  );

  registry.register(
    "flow.block",
    {
      id: `${space}.proposal`,
      match: (block: BlockShape) => proposalOf(block, module) !== null,
      render: (block: BlockShape) => {
        const found = proposalOf(block, module)!;
        return <ProposalCard id={found.id} proposal={found.proposal} module={module} />;
      },
    },
    owner,
  );
}
