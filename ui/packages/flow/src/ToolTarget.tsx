/**
 * A `flow-author` tool call that changes something, named by what it acts on (xops-02 review #4).
 *
 * The permission card under it says only "Allow flow_update?": the engine replaces the tool's own
 * question with its generic one, so the card cannot say which flow. The row the call is drawn in —
 * the `tool.row` this package registers — says it instead, read from the call's input: the flow
 * named in `flow` (the tool refuses if that is no longer the conversation's current flow), the name
 * and title of a flow being created, the skill being written.
 */

import { BuiltinToolRow, useT, type ToolRowContribution } from "@harnix/react";

import { toolTarget, type ToolTarget } from "./model";

/** A tool block, as a `tool.row` contribution is handed it. */
export type ToolBlock = Parameters<ToolRowContribution["render"]>[0];

/** The line that names the target; null for a call it has nothing to say about. */
export function ToolTargetLine({ target }: { target: ToolTarget }) {
  const t = useT();
  const what = target.tool;
  const name = target.flow ?? target.skill ?? "?";
  const text =
    what === "create"
      ? t("flowui.target.create", { flow: name, title: target.title ?? name })
      : what === "update" && target.title
        ? t("flowui.target.updateTitled", { flow: name, title: target.title })
        : what === "start" && target.subject
          ? t("flowui.target.startOn", { flow: name, subject: target.subject })
          : t("flowui.target." + what, { flow: name, skill: name });
  return (
    <div className="hx-flow-tool-target" data-testid="flow-tool-target" data-tool={what} data-flow={target.flow ?? ""} data-skill={target.skill ?? ""}>
      {text}
    </div>
  );
}

/** The tool row: the target, then the shell's ordinary row. */
export function FlowToolRow({ block }: { block: ToolBlock }) {
  const target = toolTarget(block.name, block.input);
  return (
    <>
      {target ? <ToolTargetLine target={target} /> : null}
      <BuiltinToolRow block={block} />
    </>
  );
}
