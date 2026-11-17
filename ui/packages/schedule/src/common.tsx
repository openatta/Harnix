/**
 * What the lists, the badges and the editor share: a call's error read, a target in words, and the
 * way to what a receipt points to — a session, opened where it is listed, or a task's main session.
 */

import type { Receipt, Schedule, Target } from "@harnix/core";
import { t, type HostApi, type Registry } from "@harnix/react";

/** An error's JSON-RPC code, when it has one. */
export function errorCode(e: unknown): number | undefined {
  const code = (e as { code?: unknown } | null)?.code;
  return typeof code === "number" ? code : undefined;
}

/** An error's text. */
export function errorText(e: unknown): string {
  if (e && typeof e === "object" && typeof (e as { message?: unknown }).message === "string") return (e as { message: string }).message;
  return typeof e === "string" ? e : String(e);
}

/** A directory's last part, as a project is named in a line. */
export function baseName(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  return trimmed.slice(trimmed.lastIndexOf("/") + 1) || path;
}

/** A session's name as the page knows it, else its id's head. */
export function sessionName(host: HostApi, instance: string, sessionId: string): string {
  try {
    const row = host.session.list().find((r) => r.session_id === sessionId && (r.instance ?? host.defaults.instance) === instance);
    if (row?.name) return row.name;
  } catch {
    // A host without the list: the id.
  }
  return sessionId.slice(0, 8);
}

/** A target in a line: what it does, and where. */
export function targetText(host: HostApi, target: Target): string {
  if (target.kind === "session") return t("schedule.target.session", { name: sessionName(host, target.instance, target.session_id) });
  if (target.kind === "new_session") {
    return target.project_root ? t("schedule.target.newSessionIn", { project: baseName(target.project_root) }) : t("schedule.target.newSession");
  }
  return t("schedule.target.taskIn", { project: baseName(target.project_root), flow: target.flow });
}

/** Why a delivery did not go: a skip's reason in words; a failure's error as it came. */
export function reasonText(reason: string | undefined | null): string {
  if (!reason) return "";
  return ["missed", "min_interval", "previous_run", "merged"].includes(reason) ? t(`schedule.reason.${reason}`) : reason;
}

/**
 * Open a session: in the space in front when that one lists `space`'s sessions, else in the space
 * it belongs to, brought to the front.
 */
export async function openSessionAt(host: HostApi, r: Registry, ref: { instance: string; session_id: string }, space?: string | null): Promise<void> {
  const front = host.spaces.current();
  const lists = r.at("space").find((c) => c.id === front)?.sessions ?? front;
  const session = { instance: ref.instance, sessionId: ref.session_id };
  if (space && lists === space) await host.session.open(session);
  else await host.session.open(session, { switchSpace: true });
}

/** A run's main session and the space it is shown in, from `flow.get`; tried a few times while a run just started has none yet. */
async function mainOfRun(host: HostApi, run: string, tries = 10, every = 500): Promise<{ instance: string; session_id: string; shown: string | null } | null> {
  for (let i = 0; i < tries; i++) {
    const view = await host.call<{ record?: { session_keys?: Record<string, string>; show_in_space?: string | null } }>("flow.get", { run });
    const key = view?.record?.session_keys?.["@main"];
    const at = key ? key.indexOf("/") : -1;
    if (key && at > 0) return { instance: key.slice(0, at), session_id: key.slice(at + 1), shown: view?.record?.show_in_space ?? null };
    await new Promise((resolve) => setTimeout(resolve, every));
  }
  return null;
}

/** Open what a receipt points to: the session it went to, or the main session of the run it started. */
export async function openReceipt(host: HostApi, r: Registry, schedule: Schedule, receipt: Receipt): Promise<void> {
  const to = receipt.target;
  if (!to) return;
  try {
    if (to.run) {
      const main = await mainOfRun(host, to.run);
      if (main) await openSessionAt(host, r, main, main.shown);
      return;
    }
    if (to.instance && to.session_id) await openSessionAt(host, r, { instance: to.instance, session_id: to.session_id }, schedule.space);
  } catch (e) {
    host.shell.notice(errorText(e), "error");
  }
}
