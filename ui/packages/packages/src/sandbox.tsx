/**
 * The sandbox extension processes start in, drawn in red wherever the packages are when it is not
 * on: `off` is the profile's `[ext.packages] sandbox = "off"`, for development; `unavailable` is a
 * system where none can be had, and no process starts — with why. Packages in this process (the
 * WASM carrier) start no process and say nothing of one (`null`).
 */

import type { Sandbox } from "@harnix/core";
import { useT } from "@harnix/react";

/** `packages.list`'s `runtime.sandbox`: what the host applies to each process it starts (`on`, `off` or `unavailable`). */
export type { Sandbox };

export function SandboxAlert({ sandbox }: { sandbox?: Sandbox | null }) {
  const tr = useT();
  if (!sandbox || sandbox.mode === "on") return null;
  return (
    <div className="packages-sandbox" role="alert" data-testid="packages-sandbox" data-mode={sandbox.mode}>
      {sandbox.mode === "off" ? tr("packages.sandbox.off") : tr("packages.sandbox.unavailable", { why: sandbox.why ?? "" })}
    </div>
  );
}
