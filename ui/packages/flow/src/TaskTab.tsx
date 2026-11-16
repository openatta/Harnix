/**
 * The right column's 流程 tab (`sidebar.tab`, kind `flow`, harnix-ext.md §22.7): the task whose
 * main session this is, in full — `RunPanel`. Which run: the one the opener named
 * (`params.run`), remembered for this session in this browser (a reopened page has no params);
 * failing both, the runs are searched for one whose main session this is.
 */

import { useEffect, useState } from "react";
import { useT, useTabInfo, type HostApi, type HostSessionRef as SessionRef } from "@harnix/react";

import { mainSessionOf, methodName, runsOf } from "./model";
import { RunPanel } from "./RunPanel";
import type { RunView } from "./types";

const REMEMBERED = "harnix.flow.task";
/** How many of the newest runs are read when a session's run has to be found. */
const SEARCHED = 20;

const keyOf = (session: SessionRef) => `${REMEMBERED}.${session.instance}/${session.sessionId}`;

export function rememberedRun(session: SessionRef | null): string | null {
  if (!session) return null;
  try {
    return localStorage.getItem(keyOf(session));
  } catch {
    return null;
  }
}

export function rememberRun(session: SessionRef | null, run: string): void {
  if (!session) return;
  try {
    localStorage.setItem(keyOf(session), run);
  } catch {
    // Private mode: known until the page goes.
  }
}

/** The run whose main session this is, among the newest ones; null when none is. */
export async function findRunOf(host: HostApi, module: string, session: SessionRef): Promise<string | null> {
  const runs = runsOf(await host.call(methodName(module, "list"), { limit: SEARCHED }).catch(() => null));
  for (const r of runs.slice(0, SEARCHED)) {
    try {
      const main = mainSessionOf((await host.call<RunView>(methodName(module, "get"), { run: r.id }))?.record);
      if (main && main.instance === session.instance && main.sessionId === session.sessionId) return r.id;
    } catch {
      // A run gone meanwhile: the next.
    }
  }
  return null;
}

export function TaskTab({ host, module }: { host: HostApi; module: string }) {
  const t = useT();
  const { tab } = useTabInfo();
  const session = tab.session;
  const named = typeof tab.navigation.params.run === "string" ? tab.navigation.params.run : null;
  const [run, setRun] = useState<string | null>(() => named ?? rememberedRun(session));
  const [searched, setSearched] = useState(false);
  // Each navigation names a run: that one, remembered for the session.
  useEffect(() => {
    if (named) {
      setRun(named);
      rememberRun(session, named);
    }
  }, [named, tab.navigation.revision]);
  useEffect(() => {
    if (run || !session) return;
    let live = true;
    void findRunOf(host, module, session).then((found) => {
      if (!live) return;
      if (found) {
        setRun(found);
        rememberRun(session, found);
      }
      setSearched(true);
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
      <div className="hx-flow-panel hx-flow-task-tab" data-testid="flow-task-tab" data-state={searched ? "none" : "searching"}>
        <p className="hx-flow-hint">{t(searched ? "flowui.task.noTask" : "flowui.loading")}</p>
      </div>
    );
  }
  return (
    <div className="hx-flow-panel hx-flow-task-tab" data-testid="flow-task-tab" data-run={run}>
      <RunPanel module={module} run={run} onOpenFiles={openFiles} />
    </div>
  );
}
