/**
 * A session's proposal (xops-01 §6): `flow_propose` writes an `app.<module>` entry with
 * `event: "proposal"` into the conversation, and nothing happens until a person takes it. Accept
 * calls `rework` or `cancel` with the session's reason; Dismiss only hides the card on this page.
 */

import { useState, useSyncExternalStore } from "react";
import { useNest, useT } from "@harnix/react";

import { errorText } from "./hooks";
import { methodName } from "./model";
import type { Proposal } from "./types";

/** Cards dismissed on this page, by entry id: a redraw of the conversation does not bring them back. */
const dismissed = new Set<string>();
const listeners = new Set<() => void>();
let version = 0;
function dismiss(id: string): void {
  dismissed.add(id);
  version += 1;
  for (const fn of [...listeners]) fn();
}
function subscribe(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export interface ProposalCardProps {
  /** The entry's id. */
  id: string;
  proposal: Proposal;
  module?: string;
}

export function ProposalCard({ id, proposal, module = "flow" }: ProposalCardProps) {
  const t = useT();
  const nest = useNest();
  useSyncExternalStore(subscribe, () => version);
  const [state, setState] = useState<"open" | "busy" | "accepted">("open");
  const [error, setError] = useState<string | null>(null);
  if (dismissed.has(id)) return null;

  const accept = async () => {
    setState("busy");
    setError(null);
    try {
      if (proposal.kind === "rework") {
        await nest.call(methodName(module, "rework"), { run: proposal.run, to: proposal.to, reason: proposal.reason });
      } else {
        await nest.call(methodName(module, "cancel"), { run: proposal.run, reason: proposal.reason });
      }
      setState("accepted");
    } catch (e) {
      setError(errorText(e));
      setState("open");
    }
  };

  const canAccept = proposal.kind === "cancel" || !!proposal.to;
  return (
    <div className="hx-blk hx-flow-proposal" data-testid="flow-proposal" data-reveal={id} data-kind={proposal.kind} data-state={state}>
      <div className="hx-flow-proposal-head">
        {proposal.kind === "rework"
          ? t("flowui.proposal.rework", { station: proposal.station, to: proposal.to ?? "?", run: proposal.run })
          : t("flowui.proposal.cancel", { station: proposal.station, run: proposal.run })}
      </div>
      {proposal.reason ? <div className="hx-flow-proposal-reason">{proposal.reason}</div> : null}
      {state === "accepted" ? (
        <div className="hx-flow-tone-ok" data-testid="flow-proposal-accepted">
          {t("flowui.proposal.accepted")}
        </div>
      ) : (
        <div className="hx-flow-actions">
          <button type="button" className="btn sm primary" data-testid="flow-proposal-accept" disabled={state === "busy" || !canAccept} onClick={() => void accept()}>
            {t("flowui.proposal.accept")}
          </button>
          <button type="button" className="btn sm ghost" data-testid="flow-proposal-dismiss" disabled={state === "busy"} onClick={() => dismiss(id)}>
            {t("flowui.proposal.dismiss")}
          </button>
        </div>
      )}
      {error ? (
        <div className="hx-flow-error" role="alert">
          {error}
        </div>
      ) : null}
    </div>
  );
}
