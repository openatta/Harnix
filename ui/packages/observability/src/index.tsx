/**
 * @harnix/observability — the `observability` extension's interface: two tabs of the system space's
 * right column for a session (`system.inspector.tab`) — every model call, what went in and what
 * came out, and each turn's calls summed — read from the recordings (`observability.trace`,
 * `observability.turns`).
 *
 * Where the system space is not there the point is never defined, and the tabs go with it.
 */

import { t, type HostApi, type HostSessionRef as SessionKey, type Registry } from "@harnix/react";

import { CallsTab } from "./calls.tsx";
import type { Observed } from "./shared.tsx";
import { TurnsTab } from "./turns.tsx";
import "./observability.css";

/** The backend extension this package is the interface of. */
export const extension = "observability";

/** The point the tabs go to, as the system space defines it. */
const INSPECTOR_TAB = "system.inspector.tab";

/** What an inspector tab is asked about: a session, and a tool call to pick out. */
interface Target {
  kind: "session";
  instance: string;
  sessionId: string;
  tool?: string;
}

export function register(r: Registry, host: HostApi): void {
  const observed: Observed = { host, registry: r };
  const sessionOf = (target: Target): SessionKey => ({ instance: target.instance, sessionId: target.sessionId });
  r.register(
    INSPECTOR_TAB,
    {
      id: "observability.calls",
      // The tab a person coming from a tool row lands on: it picks out the call that asked for the tool.
      tools: true,
      title: () => t("system.tab.calls"),
      match: (target: Target) => target.kind === "session",
      render: (target: Target) => <CallsTab key={`${target.instance}/${target.sessionId}/${target.tool ?? ""}`} observed={observed} session={sessionOf(target)} tool={target.tool} />,
    },
    extension,
  );
  r.register(
    INSPECTOR_TAB,
    {
      id: "observability.turns",
      title: () => t("system.tab.turns"),
      match: (target: Target) => target.kind === "session",
      render: (target: Target) => <TurnsTab host={host} session={sessionOf(target)} />,
    },
    extension,
  );
}
