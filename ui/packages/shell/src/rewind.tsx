/**
 * Going back to an earlier point of a session (`harnix.session.rewind`, docs/harnix-app.md §5.1):
 * 「回到这里」 in a person's message's menu takes the conversation and the files back to just before
 * that message, and puts the message back into the composer; 「把文件退回到这次调用之前」 on a file
 * tool's row takes the files alone back to before that call.
 *
 * A rewind writes project files, so it is never done unseen: the dialog first asks for a dry run and
 * shows the turns it would take out, the files it would restore or delete, the files changed since
 * the session wrote them, and the ones it cannot restore — then a person confirms, and decides
 * whether to fork first and whether to overwrite or skip what is in the way. Not while a turn runs
 * or waits: the engine would refuse it, and the point it goes back to is chosen looking at what is
 * on the page.
 */

import { useEffect, useState } from "react";
import { errorText, type RewindIssue, type RewindParams, type RewindResult } from "@harnix/core";
import { t, useT, type ActionTarget } from "@harnix/react";

import type { App } from "./app.ts";
import { Body, Foot } from "./dialogs.tsx";
import { hostDialogs } from "./host-dialogs.tsx";

const OWNER = "builtin";

/** The engine's file tools: what a rewind to a tool use puts back. Commands' writes are not. */
const FILE_TOOLS = new Set(["Write", "Edit", "MultiEdit", "NotebookEdit"]);

/** Why the open session cannot be rewound now; null when it can. */
function busy(app: App, target: ActionTarget): string | null {
  const current = app.snapshot.current;
  if (!current || current.instance !== target.session.instance || current.sessionId !== target.session.sessionId) return null;
  const state = app.controller?.getSnapshot();
  const running = state?.turn != null || state?.session?.turn_state === "running" || (state?.queue.length ?? 0) > 0;
  return running ? t("rewind.busy") : null;
}

export function registerRewind(app: App): void {
  const r = app.registry;
  r.register(
    "session.action",
    {
      id: "builtin.rewind",
      at: ["message"],
      icon: "refresh",
      label: () => t("rewind.here"),
      match: (target) => target.kind === "message",
      disabled: (target) => busy(app, target),
      run: (target) => (target.kind === "message" ? rewindDialog(app, target, { before_turn: target.turnId }) : undefined),
    },
    OWNER,
  );
  r.register(
    "session.action",
    {
      id: "builtin.rewind-files",
      at: ["tool"],
      icon: "refresh",
      label: () => t("rewind.files"),
      match: (target) => target.kind === "tool" && FILE_TOOLS.has(target.tool),
      disabled: (target) => busy(app, target),
      run: (target) => (target.kind === "tool" ? rewindDialog(app, target, { tool_use: target.toolUseId }) : undefined),
    },
    OWNER,
  );
}

/**
 * The dialog: a dry run, what it says, and the rewind once confirmed. Settles once it is done or
 * closed; on a rewind that took turns out, the first of them — the message it went back to — is put
 * back into the session's composer.
 */
export async function rewindDialog(app: App, target: ActionTarget, to: RewindParams["to"]): Promise<void> {
  const ask: RewindParams = {
    instance: target.session.instance,
    session_id: target.session.sessionId,
    space: app.claim(target.space),
    to,
  };
  const title = target.kind === "tool" ? t("rewind.titleFiles") : t("rewind.title");
  const done = await hostDialogs(app).open<RewindResult>((close) => <RewindBody app={app} ask={ask} close={close} />, { title, wide: true });
  if (!done) return;
  const prompt = done.conversation?.removed_turns[0]?.prompt;
  if (prompt) app.insert(prompt, target.session);
  const turns = done.conversation?.removed_turns.length ?? 0;
  const files = done.files?.restored.length ?? 0;
  app.notice(t("rewind.done", { turns, files }));
  if (done.fork_session_id) void app.refreshSessions();
}

function RewindBody({ app, ask, close }: { app: App; ask: RewindParams; close: (result: RewindResult | null) => void }) {
  const t = useT();
  const [plan, setPlan] = useState<RewindResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [fork, setFork] = useState(false);
  const [force, setForce] = useState(false);
  const [skip, setSkip] = useState(false);
  const [running, setRunning] = useState(false);

  useEffect(() => {
    let gone = false;
    app.call<RewindResult>("harnix.session.rewind", { ...ask, dry_run: true }).then(
      (p) => !gone && setPlan(p),
      (e: unknown) => !gone && setError(errorText(e)),
    );
    return () => {
      gone = true;
    };
  }, [app, ask]);

  const skipped = plan?.files?.skipped ?? [];
  const conflicts = skipped.filter((i) => i.overridable_by === "force");
  const others = skipped.filter((i) => i.overridable_by !== "force");
  const turns = plan?.conversation?.removed_turns ?? [];
  const restored = plan?.files?.restored ?? [];
  const blocked = !plan || running || (conflicts.length > 0 && !force) || (others.length > 0 && !skip);

  const confirm = async () => {
    setRunning(true);
    setError(null);
    try {
      const params: RewindParams = { ...ask, ...(fork ? { fork_first: true } : {}), ...(force ? { force: true } : {}), ...(skip ? { allow_partial: true } : {}) };
      close(await app.call<RewindResult>("harnix.session.rewind", params));
    } catch (e) {
      setError(t("rewind.failed", { error: errorText(e) }));
      setRunning(false);
    }
  };

  return (
    <>
      <Body>
        <div className="rewind" data-testid="rewind-dialog" data-state={plan ? "planned" : error ? "failed" : "planning"}>
          {!plan && !error ? <div className="hint">{t("rewind.planning")}</div> : null}
          {plan ? (
            <>
              {plan.conversation ? (
                <section data-testid="rewind-turns">
                  <h4>{t("rewind.turns", { count: turns.length })}</h4>
                  <ul>
                    {turns.map((turn) => (
                      <li key={turn.turn_id} data-turn={turn.turn_id}>
                        {turn.prompt.trim() ? oneLine(turn.prompt) : turn.turn_id}
                      </li>
                    ))}
                  </ul>
                </section>
              ) : null}
              {plan.files ? (
                <section data-testid="rewind-restore">
                  <h4>{t("rewind.restore", { count: restored.length })}</h4>
                  {restored.length ? (
                    <ul>
                      {restored.map((file) => (
                        <li key={file.path} data-path={file.path} data-to={file.to}>
                          <code>{file.path}</code>
                          {file.to === "absent" ? <span className="hint"> · {t("rewind.deleted")}</span> : null}
                          {file.outside_project ? <span className="hint"> · {t("rewind.outside")}</span> : null}
                        </li>
                      ))}
                    </ul>
                  ) : (
                    <div className="hint">{t("rewind.noFiles")}</div>
                  )}
                </section>
              ) : null}
              <Issues testId="rewind-conflicts" title={t("rewind.conflicts", { count: conflicts.length })} issues={conflicts} />
              <Issues testId="rewind-issues" title={t("rewind.issues", { count: others.length })} issues={others} />
              <div className="hint">{t("rewind.commandsNote")}</div>
              {conflicts.length ? (
                <label className="check">
                  <input type="checkbox" data-testid="rewind-force" checked={force} onChange={(e) => setForce(e.target.checked)} />
                  {t("rewind.force")}
                </label>
              ) : null}
              {others.length ? (
                <label className="check">
                  <input type="checkbox" data-testid="rewind-skip" checked={skip} onChange={(e) => setSkip(e.target.checked)} />
                  {t("rewind.skip")}
                </label>
              ) : null}
              <label className="check">
                <input type="checkbox" data-testid="rewind-fork" checked={fork} onChange={(e) => setFork(e.target.checked)} />
                {t("rewind.fork")}
              </label>
            </>
          ) : null}
          {error ? (
            <div className="hint err" data-testid="rewind-error" role="alert">
              {error}
            </div>
          ) : null}
        </div>
      </Body>
      <Foot>
        <button type="button" className="btn outline" data-testid="rewind-cancel" onClick={() => close(null)}>
          {t("common.cancel")}
        </button>
        <button type="button" className="btn danger" data-testid="rewind-confirm" disabled={blocked} onClick={() => void confirm()}>
          {t("rewind.confirm")}
        </button>
      </Foot>
    </>
  );
}

function Issues({ testId, title, issues }: { testId: string; title: string; issues: RewindIssue[] }) {
  if (!issues.length) return null;
  return (
    <section data-testid={testId}>
      <h4>{title}</h4>
      <ul>
        {issues.map((issue) => (
          <li key={`${issue.path}:${issue.issue}`} data-path={issue.path} data-issue={issue.issue}>
            <code>{issue.path}</code>
            {issue.detail ? <span className="hint"> · {issue.detail}</span> : null}
          </li>
        ))}
      </ul>
    </section>
  );
}

/** A prompt as one line: its first, cut. */
function oneLine(text: string): string {
  const first = text.trim().split("\n")[0] ?? "";
  return first.length > 120 ? `${first.slice(0, 120)}…` : first;
}
