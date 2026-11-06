/**
 * `registerFlowSpace()`: the flow module's components put together as one space (xops-02 §7). The
 * shell registers it itself when the backend has the flow module; an application that builds its
 * own shell may call it too.
 *
 *   left    the space's conversations (the shell's own list)
 *   centre  the conversation of what is selected on the right (xops-03 §6): a flow's own
 *           (`flow-author`, whose tools write and run it), a run's main session, or the general
 *           one; the line over the composer names its flow                (`composerNotice`)
 *   right   flows · runs · reference: the lists, shown, never edited here    (`tabs`)
 *   beside  the selected flow's or run's details, between the centre and the right column
 *           (`secondary`, xops-04 §7); closing it lets go of the selection there
 *
 * An empty conversation says what it is for (`emptyConversation`).
 *
 * See `FlowSpaceState`. Also the `flow.block` that draws a session's proposal, and the `flowui`
 * strings.
 */

import { t as translate, type HostSessionRef, type NestHost, type Registry } from "@harnix/react";

import { actorLabel, proposalHandledOf, proposalOf, type BlockShape } from "./model";
import { ProposalCard } from "./ProposalCard";
import { FlowSpaceState } from "./state";
import { registerFlowStrings, STRINGS_OWNER } from "./strings";
import { EmptyConversation, FlowSecondary, FlowsTab, FocusNotice, ReferenceTab, RunsTab, type FlowTabs } from "./tabs";
import { FlowToolRow, type ToolBlock } from "./ToolTarget";
import { toolTarget } from "./model";

export interface FlowSpaceOptions {
  /** The space's id — the backend space whose conversations run `flow-author`; "flow" by default. */
  space?: string;
  /** The id the application registered the flow module under; "flow" by default (`app.flow.*`). */
  module?: string;
  /** The space's title; the `flowui.space` string by default. */
  title?: string;
  icon?: string;
  order?: number;
  group?: string;
  /** The owner the contributions are registered as; "flowui" by default. */
  owner?: string;
  /** Given: the middle follows the selection from now on, not only once the space is first drawn. */
  host?: NestHost;
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
  const state = new FlowSpaceState(space, module, tabs);
  if (options.host) state.attach(options.host);

  registerFlowStrings(registry);

  registry.register(
    "space",
    {
      id: space,
      icon: options.icon ?? "fork",
      order: options.order ?? 60,
      ...(options.group !== undefined ? { group: options.group } : {}),
      title: () => options.title ?? translate("flowui.space"),
      subtitle: () => translate("flowui.space.subtitle"),
      center: "chat",
      details: "open",
      detailsWidth: 400,
      placeholder: () => translate("flowui.composer.placeholder"),
      notice: () => {
        const flow = state.get().flow;
        return flow ? translate("flowui.focus.next", { flow }) : translate("flowui.focus.none");
      },
      composerNotice: (session: HostSessionRef) => <FocusNotice state={state} session={session} />,
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
      tabs: [
        { id: tabs.flows, title: () => translate("flowui.tab.flows"), render: () => <FlowsTab state={state} tabs={tabs} /> },
        { id: tabs.runs, title: () => translate("flowui.tab.runs"), render: () => <RunsTab state={state} tabs={tabs} /> },
        { id: tabs.reference, title: () => translate("flowui.tab.reference"), render: () => <ReferenceTab state={state} />, align: "end" },
      ],
    },
    owner,
  );

  // A call of the conversation's tools that changes something, named by what it acts on: the
  // permission card under it cannot say (it reads "Allow flow_update?").
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
      match: (block: BlockShape) => proposalHandledOf(block, `app.${module}`) !== null,
      render: (block: BlockShape) => {
        const h = proposalHandledOf(block, `app.${module}`)!;
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
      match: (block: BlockShape) => proposalOf(block, `app.${module}`) !== null,
      render: (block: BlockShape) => {
        const found = proposalOf(block, `app.${module}`)!;
        return <ProposalCard id={found.id} proposal={found.proposal} module={module} />;
      },
    },
    owner,
  );

  return { state, space, module, tabs };
}
