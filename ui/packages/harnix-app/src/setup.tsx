/**
 * The Harnix app's first-run steps, on the new conversation's empty page (`home.card`): a project
 * to run conversations in, and what a new conversation starts with — its scene and permission
 * mode — with `code` offered beside the default. Each is drawn until it is done or closed
 * (`SetupStep`); the model's step is the settings'.
 *
 * Nothing here is about trust: a project that is not trusted is said over its conversation
 * (`TrustBanner`), where its settings are held back, not on the empty page.
 */

import { useEffect, useState } from "react";
import type { Scene } from "@harnix/core";
import { SetupStep, dismissSetupStep, setupStepDismissed, useHello, useSetupStep, useT, type HostApi } from "@harnix/react";

import { addWorkspaceDialog, newSessionDialog } from "./dialogs.tsx";
import { useApp } from "./sessions.tsx";
import type { AppStore } from "./store.ts";

export const PROJECT_STEP = "harnix-app.setup.project";
export const SCENE_STEP = "harnix-app.setup.scene";

/** The engine's coding scene: offered on the scene step when it is served and not the default. */
const CODE = "code";

const MODES = ["default", "acceptEdits", "plan", "bypassPermissions"];

/** Done once there is a project; until the list is read, not known. */
export function ProjectStep({ host, store }: { host: HostApi; store: AppStore }) {
  const t = useT();
  const s = useApp(store);
  const state = useSetupStep(PROJECT_STEP, s.loaded ? s.workspaces.length > 0 : null);
  const first = s.workspaces[0];
  const name = first ? (first.title ?? first.path.replace(/\/+$/, "").split("/").pop() ?? first.path) : "";
  return (
    <SetupStep
      id={PROJECT_STEP}
      state={state}
      icon="folder"
      title={t("setup.project.title")}
      actions={
        state === "todo" ? (
          <button type="button" className="btn sm primary" data-testid="setup-project-add" onClick={() => void addWorkspaceDialog(host, store)}>
            {t("setup.project.action")}
          </button>
        ) : first ? (
          <button
            type="button"
            className="btn sm outline"
            data-testid="setup-project-open"
            onClick={() => {
              store.openProject(first.id);
              void host.spaces.switch("projects").catch(() => undefined);
            }}
          >
            {t("setup.project.open")}
          </button>
        ) : null
      }
    >
      {state === "todo" ? t("setup.project.todo") : t("setup.project.done", { name })}
    </SetupStep>
  );
}

/**
 * What a new conversation starts with: the engine's default scene, and the permission mode the
 * engine's settings give it (`permission_mode`; `default` when they cannot be read). Done when it
 * is read and closed ("got it").
 */
export function SceneStep({ host, store }: { host: HostApi; store: AppStore }) {
  const t = useT();
  const hello = useHello();
  const [mode, setMode] = useState("default");
  const sceneId = hello?.engine?.scene ?? hello?.engine?.active_scenes?.[0] ?? null;
  useEffect(() => {
    if (!sceneId || setupStepDismissed(SCENE_STEP)) return;
    let alive = true;
    host
      .call<{ fields?: { key: string; effective?: unknown }[] }>("settings.describe", { scene: sceneId, project_root: null })
      .then((doc) => {
        const value = doc?.fields?.find((f) => f.key === "permission_mode")?.effective;
        if (alive && typeof value === "string" && MODES.includes(value)) setMode(value);
      })
      // No settings to read (the product left them out): the engine's own default stands.
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [host, sceneId]);
  const state = useSetupStep(SCENE_STEP, hello && sceneId ? false : null);
  const scenes = (hello?.scenes ?? []) as Scene[];
  const scene = scenes.find((s) => s.scene === sceneId);
  const code = sceneId !== CODE ? scenes.find((s) => s.scene === CODE && s.active !== false) : undefined;
  const sceneName = scene?.name || sceneId || "";
  return (
    <SetupStep
      id={SCENE_STEP}
      state={state}
      icon="agent"
      title={t("setup.scene.title")}
      actions={
        <>
          {code ? (
            <button type="button" className="btn sm primary" data-testid="setup-scene-code" onClick={() => void newSessionDialog(host, store, undefined, CODE)}>
              {t("setup.scene.codeAction", { name: code.name || CODE })}
            </button>
          ) : null}
          <button type="button" className="btn sm outline" data-testid="setup-scene-new" onClick={() => void newSessionDialog(host, store)}>
            {t("setup.scene.choose")}
          </button>
          <button type="button" className="btn sm ghost" data-testid="setup-scene-settings" onClick={() => host.settings.open("engine")}>
            {t("setup.scene.change")}
          </button>
          <button type="button" className="btn sm ghost" data-testid="setup-scene-got" onClick={() => dismissSetupStep(SCENE_STEP)}>
            {t("setup.scene.got")}
          </button>
        </>
      }
    >
      <div className="line" data-testid="setup-scene-default" data-scene={sceneId ?? undefined}>
        {scene?.description ? t("setup.scene.scene", { name: sceneName, description: scene.description }) : t("setup.scene.sceneBare", { name: sceneName })}
      </div>
      <div className="line" data-testid="setup-scene-mode" data-mode={mode}>
        {t("setup.scene.mode", { mode, what: t(`setup.mode.${mode}`) })}
      </div>
      {code ? (
        <div className="line" data-testid="setup-scene-offer">
          {t("setup.scene.code", { name: code.name || CODE })}
        </div>
      ) : null}
    </SetupStep>
  );
}
