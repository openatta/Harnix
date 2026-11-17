/**
 * The right column's 流程 tab (`sidebar.tab`, kind `flow`, harnix-ext.md §22.7): the task whose
 * main session this is, in full — `RunPanel`. Which run: the one the opener named
 * (`params.run`); failing that (a reopened page has no params), the module is asked whose main
 * session this is (`flow.runOf`).
 */

import { useEffect, useState } from "react";
import { useT, useTabInfo, type HostApi, type HostSessionRef as SessionRef } from "@harnix/react";

import { methodName } from "./model";
import { RunPanel } from "./RunPanel";

/** The run whose main session this is, as the module says (`flow.runOf`); null when it is no run's. */
export async function runOf(host: HostApi, module: string, session: SessionRef): Promise<string | null> {
  try {
    const answer = await host.call<{ run?: string | null }>(methodName(module, "runOf"), { instance: session.instance, session_id: session.sessionId });
    return typeof answer?.run === "string" && answer.run ? answer.run : null;
  } catch {
    return null;
  }
}

export function TaskTab({ host, module }: { host: HostApi; module: string }) {
  const t = useT();
  const { tab } = useTabInfo();
  const session = tab.session;
  const named = typeof tab.navigation.params.run === "string" ? tab.navigation.params.run : null;
  const [run, setRun] = useState<string | null>(named);
  const [asked, setAsked] = useState(false);
  // Each navigation names a run: that one.
  useEffect(() => {
    if (named) setRun(named);
  }, [named, tab.navigation.revision]);
  useEffect(() => {
    if (run || !session) return;
    let live = true;
    void runOf(host, module, session).then((found) => {
      if (!live) return;
      if (found) setRun(found);
      setAsked(true);
    });
    return () => {
      live = false;
    };
  }, [run, session?.instance, session?.sessionId]);
  const openFiles = () => {
    try {
      tab.actions.openTab("files");
      return true;
    } catch {
      return false;
    }
  };
  if (!run) {
    return (
      <div className="hx-flow-panel hx-flow-task-tab" data-testid="flow-task-tab" data-state={asked ? "none" : "searching"}>
        <p className="hx-flow-hint">{t(asked ? "flowui.task.noTask" : "flowui.loading")}</p>
      </div>
    );
  }
  return (
    <div className="hx-flow-panel hx-flow-task-tab" data-testid="flow-task-tab" data-run={run}>
      <RunPanel module={module} run={run} onOpenFiles={openFiles} />
    </div>
  );
}
