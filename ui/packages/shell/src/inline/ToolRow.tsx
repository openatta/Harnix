/**
 * `tool.row`, declared: a call of an application's tool drawn with the shell's own row, as its
 * manifest's `[[inline.tool_row]]` says — the glyph, the title (from the call's input), what
 * follows it (the result, or a field of the input), and whether the result shows unfolded.
 */

import type { BlockOf } from "@harnix/core/state";
import { Disclosure, STATUS_GLYPH, ToolDetails, clip, t, toolArgs, useToolState } from "@harnix/react";

import type { App } from "../app.ts";
import { Glyph } from "../glyph.tsx";
import type { RuntimeApp, ToolRowDecl } from "./apps.ts";
import { inputOf, lookup, say, words } from "./template.ts";

/** The row's title for a call: the declared one, filled from the call's input; else the tool's name. */
export function inlineTitle(decl: ToolRowDecl, block: BlockOf<"tool">): string {
  return decl.title ? say(decl.title, { input: inputOf(block.input), result: block.result ?? "" }) : decl.tool;
}

export function InlineToolRow({ app, owner, decl, block }: { app: App; owner: RuntimeApp; decl: ToolRowDecl; block: BlockOf<"tool"> }) {
  const scope = { input: inputOf(block.input), result: block.result ?? "" };
  const title = inlineTitle(decl, block);
  const summary =
    decl.summary === "result"
      ? block.status === "running"
        ? ""
        : words(block.result)
      : decl.summary
        ? words(lookup(scope, decl.summary))
        : toolArgs(block.input);
  const { state, tail } = useToolState(block);
  return (
    <div className="app-inline" data-inline="tool.row" data-app={owner.id} data-tool={decl.tool}>
      <Disclosure
        testId="conv-tool"
        id={block.id}
        state={state}
        initiallyOpen={decl.collapsed === false}
        summary={
          <>
            <span className={`hx-status hx-status-${block.status}`} aria-hidden>
              {STATUS_GLYPH[block.status]}
            </span>
            <Glyph name={decl.icon ?? "terminal"} className="app-inline-icon" />
            <span className="hx-tool-name" data-testid="conv-tool-name" title={`${block.name} · ${owner.title}`}>
              {title}
            </span>
            <span className="hx-tool-arg" data-testid="conv-tool-summary">
              {clip(summary.replace(/\s+/g, " "), 160)}
            </span>
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
    </div>
  );
}
