/** A session's goal in the right column, and what may be done with it from here. */

import { useState } from "react";
import { errorText } from "@harnix/core";
import { useT } from "@harnix/react";

import type { App, SessionKey } from "../../app.ts";
import { Body, confirmDialog, Foot } from "../../dialogs.tsx";
import type { Goal } from "../types.ts";
import { act } from "../view.ts";

/**
 * A session's goal, and what a person may do with it from here (Agix `session.setGoal` /
 * `updateGoal`, as the system module): set one when there is none — the engine starts its first
 * round at once on an idle session — and pause, resume, edit or clear it.
 */
export function GoalBlock({ app, session, goal, changed }: { app: App; session: SessionKey; goal: Goal | null; changed: () => void }) {
  const tr = useT();
  const target = { instance: session.instance, session_id: session.sessionId };
  const update = (action: string, extra: Record<string, unknown> = {}) =>
    void act(app, () => app.call("harnix.system.goal.update", { ...target, action, ...extra })).then(changed);
  const edit = (current: Goal | null) =>
    app.openModal({
      title: tr(current ? "system.goal.edit" : "system.goal.set"),
      content: (
        <GoalForm
          goal={current}
          save={async (objective, rounds) => {
            await (current
              ? app.call("harnix.system.goal.update", { ...target, action: "edit", objective, max_rounds: rounds })
              : app.call("harnix.system.goal.set", { ...target, objective, max_rounds: rounds }));
            app.closeModal();
            changed();
          }}
          cancel={() => app.closeModal()}
        />
      ),
    });
  if (!goal) {
    return (
      <button type="button" className="btn sm outline wide" data-testid="agent-goal-set" onClick={() => edit(null)}>
        {tr("system.goal.set")}
      </button>
    );
  }
  const reason = typeof goal.blocked_reason === "string" ? goal.blocked_reason : goal.blocked_reason?.message;
  const running = goal.phase === "active" && goal.armed;
  return (
    <div className="goal" data-testid="agent-goal" data-phase={goal.phase} data-armed={goal.armed ? "" : undefined}>
      <div className="label">
        {tr("system.goal")}
        <span className={`kd phase ${goal.phase}`}>{tr(`system.goal.${goal.phase}`)}</span>
        {goal.phase === "active" && !goal.armed ? <span className="kd">{tr("system.goal.disarmed")}</span> : null}
      </div>
      <div className="objective">{goal.objective}</div>
      <div className="note">
        {tr("system.goal.rounds", { count: goal.rounds_started, of: goal.max_rounds })}
        {reason ? ` · ${reason}` : ""}
      </div>
      {goal.phase === "complete" ? null : (
        <div className="ops">
          {running ? (
            <button type="button" className="btn sm outline" data-testid="agent-goal-pause" onClick={() => update("pause")}>
              {tr("system.goal.pause")}
            </button>
          ) : (
            <button type="button" className="btn sm outline" data-testid="agent-goal-resume" onClick={() => update("resume")}>
              {tr("system.goal.resume")}
            </button>
          )}
          <button type="button" className="btn sm outline" data-testid="agent-goal-edit" onClick={() => edit(goal)}>
            {tr("system.goal.edit")}
          </button>
          <button
            type="button"
            className="btn sm outline danger"
            data-testid="agent-goal-clear"
            onClick={() =>
              confirmDialog(app, tr("system.goal.confirmClear"), async () => {
                await app.call("harnix.system.goal.update", { ...target, action: "clear" });
                app.closeModal();
                changed();
              })
            }
          >
            {tr("system.goal.clear")}
          </button>
        </div>
      )}
    </div>
  );
}

function GoalForm({ goal, save, cancel }: { goal: Goal | null; save: (objective: string, rounds: number) => Promise<void>; cancel: () => void }) {
  const tr = useT();
  const [objective, setObjective] = useState(goal?.objective ?? "");
  const [rounds, setRounds] = useState(String(goal?.max_rounds ?? 256));
  const [error, setError] = useState<string | null>(null);
  const go = async () => {
    const n = Number(rounds);
    if (!objective.trim() || !Number.isInteger(n) || n < 1) return setError(tr("system.goal.invalid"));
    try {
      await save(objective.trim(), n);
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <>
      <Body>
        <div className="field">
          <label>{tr("system.goal.objective")}</label>
          <input type="text" data-testid="goal-objective" value={objective} onChange={(e) => setObjective(e.target.value)} />
        </div>
        <div className="field">
          <label>{tr("system.goal.maxRounds")}</label>
          <input type="text" inputMode="numeric" data-testid="goal-rounds" value={rounds} onChange={(e) => setRounds(e.target.value)} />
          <div className="hint">{tr(goal ? "system.goal.editHint" : "system.goal.setHint")}</div>
        </div>
        {error ? <div className="hint err" data-testid="goal-error">{error}</div> : null}
      </Body>
      <Foot>
        <button type="button" className="btn outline" onClick={cancel}>
          {tr("common.cancel")}
        </button>
        <button type="button" className="btn primary" data-testid="goal-save" onClick={() => void go()}>
          {tr("common.save")}
        </button>
      </Foot>
    </>
  );
}
