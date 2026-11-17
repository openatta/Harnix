/**
 * A refused settings write in words a person can act on. Two refusals are about the place written
 * to rather than the value: a tier another process holds (the engine waited for it and wrote
 * nothing), and a project whose folder is not there. Those are said plainly; anything else as the
 * backend said it.
 */

import { ENGINE_PROJECT_NOT_FOUND, ENGINE_SETTINGS_LOCKED, errorText } from "@harnix/core";
import { t } from "@harnix/react";

export function settingsError(e: unknown): string {
  const code = e && typeof e === "object" ? (e as { code?: unknown }).code : undefined;
  if (code === ENGINE_SETTINGS_LOCKED) return t("settings.error.locked");
  if (code === ENGINE_PROJECT_NOT_FOUND) return t("settings.error.projectNotFound");
  return errorText(e);
}

/** The notice for a write that did not happen. */
export function writeFailed(e: unknown): string {
  return t("settings.writeFailed", { error: settingsError(e) });
}
