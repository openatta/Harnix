/** The system space's left column: the instances. */

import { useT } from "@harnix/react";

import { statusReason, type InstanceRow } from "./types.ts";
import { isSelected, useRunning, useSelection, useTopic, type View } from "./view.ts";

export function InstanceList({ view }: { view: View }) {
  const tr = useT();
  const picked = useSelection(view);
  const listed = useTopic<{ instances: InstanceRow[] }>(view, "harnix.instances");
  const running = useRunning(view);
  return (
    <div className="system-tree" data-testid="system-tree">
      <div className="sec">
        <span className="truncate">{tr("system.instances")}</span>
      </div>
      {(listed?.instances ?? []).map((instance) => {
        const turning = [...running].filter((k) => k.startsWith(`${instance.id}/`)).length;
        return (
          <button
            type="button"
            key={instance.id}
            className={`srow inst${isSelected(picked, { kind: "instance", instance: instance.id }) ? " on" : ""}`}
            data-testid="system-tree-instance"
            data-instance={instance.id}
            data-state={instance.status.state}
            title={statusReason(instance.status) ?? instance.status.state}
            onClick={() => view.select({ kind: "instance", instance: instance.id })}
          >
            <span className={`dot ${instance.status.state}`} />
            <span className="tt name">{instance.id}</span>
            <span className="meta carrier">{turning || ""}</span>
          </button>
        );
      })}
    </div>
  );
}
