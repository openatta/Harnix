/** Out of the system space, back to a session where it belongs. */

import type { HostSessionRef as SessionKey } from "@harnix/react";

import { SYSTEM } from "./constants.ts";
import type { View } from "./view.ts";

/** Back from the system space to a session where it belongs: the interface space that lists its
 *  backend space (the session space lists by time and by project, so a session of one backend
 *  space may be shown by another). */
export function openWhereItBelongs(view: View, session: SessionKey, space: string): void {
  void view.host.navigate.open({ space: view.spaceFor(space, SYSTEM), session }).catch((e: unknown) => view.host.shell.notice(e instanceof Error ? e.message : String(e), "error"));
}
