/** The system space's left column: the instances. */

import { useT } from "@harnix/react";

import type { App } from "../app.ts";
import { useShell } from "../hooks.ts";
import type { InstanceRow } from "./types.ts";
import { isSelected, useSelection, useTopic, type View } from "./view.ts";

export function InstanceList({ app, view }: { app: App; view: View }) {
  const tr = useT();
  const picked = useSelection(view);
  const listed = useTopic<{ instances: InstanceRow[] }>(view, "harnix.instances");
  const s = useShell(app);
  return (
    <div className="system-tree" data-testid="system-tree">
      <div className="sec">
        <span className="truncate">{tr("system.instances")}</span>
      </div>
      {(listed?.instances ?? []).map((instance) => {
        const running = Object.keys(s.running).filter((k) => k.startsWith(`${instance.id}/`)).length;
        return (
          <button
            type="button"
            key={instance.id}
            className={`srow inst${isSelected(picked, { kind: "instance", instance: instance.id }) ? " on" : ""}`}
            data-testid="system-tree-instance"
            data-instance={instance.id}
            data-state={instance.status.state}
            title={instance.status.reason ?? instance.status.state}
            onClick={() => view.select({ kind: "instance", instance: instance.id })}
          >
            <span className={`dot ${instance.status.state}`} />
            <span className="tt name">{instance.id}</span>
            <span className="meta carrier">{running || ""}</span>
          </button>
        );
      })}
    </div>
  );
}
