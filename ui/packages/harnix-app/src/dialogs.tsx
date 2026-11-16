/**
 * The Harnix app's dialogs: a new session (scene, permission mode, project), adding a project, a
 * session's export, trusting a project, and the directory picker the first two share; and a
 * project's menu, which opens them. Each goes through the shell's layer (`host.dialogs`), so focus, Escape and the page
 * made inert under it are the shell's; a dialog draws its own body and buttons so the buttons can
 * read its state.
 */

import { useEffect, useRef, useState } from "react";
import { errorText, type Scene } from "@harnix/core";
import { DialogBody, DialogFoot, Glyph, t, useHello, useT, type HostApi, type MenuItem } from "@harnix/react";

import type { ProjectItemContext, ProjectTrust } from "./items.ts";
import { SESSIONS, type AppStore, type TrustItem, type TrustStatus, type Workspace } from "./store.ts";

/** Run a call; a failure is said in the banner, in `failed`'s words when given. */
async function attempt(host: HostApi, run: () => Promise<unknown>, failed?: (error: string) => string): Promise<boolean> {
  try {
    await run();
    return true;
  } catch (e) {
    host.shell.notice(failed ? failed(errorText(e)) : errorText(e), "error");
    return false;
  }
}

type Close = (result: true | null) => void;

/* ── directory picker ─────────────────────────────────────────────────── */

interface Listing {
  path: string;
  breadcrumbs: { name: string; path: string }[];
  entries: { name: string; hidden?: boolean }[];
}

export function DirectoryPicker({ host, onPick }: { host: HostApi; onPick: (path: string) => void }) {
  const t = useT();
  const [listing, setListing] = useState<Listing | null>(null);
  const open = async (path?: string) => {
    await attempt(host, async () => setListing(await host.call<Listing>("harnix-app.listDirectory", path ? { path } : {})));
  };
  useEffect(() => {
    void open();
  }, []);
  if (!listing) return <div className="picker" />;
  const entries = listing.entries.filter((entry) => !entry.hidden);
  return (
    <div className="picker">
      <div className="crumbs">
        {listing.breadcrumbs.map((crumb, i) => (
          <span key={crumb.path}>
            <button type="button" onClick={() => void open(crumb.path)}>
              {crumb.name}
            </button>
            {i < listing.breadcrumbs.length - 1 ? <span>/</span> : null}
          </span>
        ))}
        <span> </span>
        <button type="button" className="btn sm outline" data-testid="picker-use" onClick={() => onPick(listing.path)}>
          {t("dialog.useThisDirectory")}
        </button>
      </div>
      <div className="list">
        {entries.map((entry) => (
          <button type="button" key={entry.name} className="entry" onClick={() => void open(`${listing.path.replace(/\/$/, "")}/${entry.name}`)}>
            <Glyph name="folder" className="glyph" />
            <span className="truncate">{entry.name}</span>
          </button>
        ))}
        {!entries.length ? <div className="empty">{t("dialog.noSubdirectories")}</div> : null}
      </div>
    </div>
  );
}

/* ── new session ──────────────────────────────────────────────────────── */

interface Recents {
  projects?: { project_root: string }[];
  cwd?: string | null;
  projects_root?: string | null;
}

/** The new-session dialog; `preselect` is the project it opens on, `scene` the scene. */
export async function newSessionDialog(host: HostApi, store: AppStore, preselect?: string, scene?: string): Promise<void> {
  let recents: Recents = {};
  try {
    recents = await host.call<Recents>("harnix-app.recentProjects", {});
  } catch {
    /* the picker still works without recents */
  }
  await host.dialogs.open<true>((close) => <NewSession host={host} store={store} recents={recents} preselect={preselect} initialScene={scene} close={close} />, { title: t("dialog.newSession") });
}

function NewSession({
  host,
  store,
  recents,
  preselect,
  initialScene,
  close,
}: {
  host: HostApi;
  store: AppStore;
  recents: Recents;
  preselect?: string;
  initialScene?: string;
  close: Close;
}) {
  const t = useT();
  const hello = useHello();
  const scenes = (hello?.scenes ?? []) as (Scene & { capabilities?: { requires_project?: boolean } })[];
  const active = scenes.filter((scene) => scene.active);
  // The engine's own default first: the active list is sorted by id, and a flow's wrapped scenes
  // (`flow-*`) sort before most.
  const preferred = hello?.engine?.scene;
  const [picked, setScene] = useState<string | null>(initialScene ?? null);
  const scene = picked ?? active.find((s) => s.scene === preferred)?.scene ?? active[0]?.scene ?? "harnix";
  const [mode, setMode] = useState("default");
  const [root, setRoot] = useState<string | null>(preselect ?? recents.cwd ?? null);
  const chosen = scenes.find((s) => s.scene === scene);
  const optional = chosen?.capabilities?.requires_project === false;

  const recentRows: { label: string; path: string | null; sub?: string }[] = [];
  if (recents.projects_root) recentRows.push({ label: recents.projects_root, path: recents.projects_root, sub: t("dialog.projectsRoot") });
  if (recents.cwd && recents.cwd !== recents.projects_root) recentRows.push({ label: recents.cwd, path: recents.cwd, sub: t("dialog.currentDirectory") });
  for (const project of recents.projects ?? []) {
    if (project.project_root !== recents.cwd && project.project_root !== recents.projects_root) {
      recentRows.push({ label: project.project_root, path: project.project_root });
    }
  }
  recentRows.push({ label: t("dialog.noProjectSession"), path: null });

  const create = () =>
    attempt(
      host,
      async () => {
        await host.session.create({ scene, project_root: root, options: { permission_mode: mode } });
        void store.refresh();
        close(true);
      },
      (error) => t("dialog.createFailed", { error }),
    );

  return (
    <>
      <DialogBody>
        <div className="field">
          <label>{t("dialog.scene")}</label>
          <select data-testid="new-scene" value={scene} onChange={(e) => setScene(e.target.value)}>
            {scenes.map((s) => (
              <option key={s.scene} value={s.scene}>
                {[s.name || s.scene, s.capabilities?.requires_project ? t("dialog.sceneNeedsProject") : "", s.active ? "" : t("dialog.sceneInactive")].join("")}
              </option>
            ))}
          </select>
        </div>
        <div className="field">
          <label>{t("dialog.permissionMode")}</label>
          <select data-testid="new-mode" value={mode} onChange={(e) => setMode(e.target.value)}>
            <option value="default">{t("dialog.mode.default")}</option>
            <option value="acceptEdits">{t("dialog.mode.acceptEdits")}</option>
            <option value="plan">{t("dialog.mode.plan")}</option>
            <option value="bypassPermissions">{t("dialog.mode.bypass")}</option>
          </select>
        </div>
        <div className={`field${optional ? " optional" : ""}`}>
          <label>{t("dialog.projectRoot")}</label>
          <div className="menu-list">
            {recentRows.map((row) => (
              <button type="button" key={row.label} onClick={() => setRoot(row.path)}>
                <Glyph name="folder" className="glyph" />
                <span className="truncate">{row.label}</span>
                {row.sub ? <span className="sub">{row.sub}</span> : null}
              </button>
            ))}
          </div>
          <label>{t("dialog.orBrowse")}</label>
          <DirectoryPicker host={host} onPick={setRoot} />
          <div className="picked" data-testid="new-root">
            {root ?? t("dialog.noProject")}
          </div>
        </div>
      </DialogBody>
      <DialogFoot>
        <button type="button" className="btn outline" onClick={() => close(null)}>
          {t("common.cancel")}
        </button>
        <button type="button" className="btn primary" data-testid="new-create" onClick={() => void create()}>
          {t("common.create")}
        </button>
      </DialogFoot>
    </>
  );
}

/* ── add project ──────────────────────────────────────────────────────── */

export async function addWorkspaceDialog(host: HostApi, store: AppStore): Promise<void> {
  await host.dialogs.open<true>((close) => <AddWorkspace host={host} store={store} close={close} />, { title: t("dialog.addProject") });
}

function AddWorkspace({ host, store, close }: { host: HostApi; store: AppStore; close: Close }) {
  const t = useT();
  const [name, setName] = useState("");
  const [chosen, setChosen] = useState<string | null>(null);
  const add = async () => {
    if (!name.trim() && !chosen) {
      host.shell.notice(t("dialog.pickDirectoryFirst"));
      return;
    }
    const ok = await attempt(
      host,
      () => (name.trim() ? host.call("harnix-app.projects.create", { name: name.trim() }) : host.call("harnix-app.workspaces.create", { path: chosen })),
      (error) => t("dialog.addFailed", { error }),
    );
    if (ok) {
      close(true);
      void store.refresh();
    }
  };
  return (
    <>
      <DialogBody>
        <div className="field">
          <label>{t("dialog.newProject")}</label>
          <input
            type="text"
            placeholder={t("dialog.projectNamePlaceholder")}
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void add();
            }}
          />
          <div className="hint">{t("dialog.newProjectHint")}</div>
        </div>
        <div className="field">
          <label>{t("dialog.existingProject")}</label>
          <DirectoryPicker host={host} onPick={setChosen} />
          <div className="picked">{chosen ?? t("dialog.noDirectoryChosen")}</div>
        </div>
      </DialogBody>
      <DialogFoot>
        <button type="button" className="btn outline" onClick={() => close(null)}>
          {t("common.cancel")}
        </button>
        <button type="button" className="btn primary" data-testid="project-add" onClick={() => void add()}>
          {t("common.add")}
        </button>
      </DialogFoot>
    </>
  );
}

/* ── export ───────────────────────────────────────────────────────────── */

/** Where a session is, and the backend space a call about it claims. */
export interface ExportTarget {
  instance: string;
  session_id: string;
  space: string;
}

/**
 * Export a session: as markdown to read, or as the engine's archive to keep (the session with its
 * sub-agents' sessions, the images in its messages, its entries and a manifest — and the recorded
 * model calls only when asked, since they carry the full prompts and tool input). The file is made
 * here from what the backend answers.
 */
export async function exportDialog(host: HostApi, target: ExportTarget): Promise<void> {
  await host.dialogs.open<true>((close) => <ExportForm host={host} target={target} close={close} />, { title: t("dialog.export") });
}

function ExportForm({ host, target, close }: { host: HostApi; target: ExportTarget; close: Close }) {
  const t = useT();
  const [format, setFormat] = useState<"markdown" | "archive">("markdown");
  const [recording, setRecording] = useState(false);
  const go = () =>
    attempt(
      host,
      async () => {
        const answer = await host.call<{ file: string; format: string; content: unknown; encoding?: string }>("harnix-app.sessions.export", {
          ...target,
          format,
          recording: format === "archive" && recording,
        });
        const body =
          answer.encoding === "base64"
            ? Uint8Array.from(atob(String(answer.content)), (c) => c.charCodeAt(0))
            : typeof answer.content === "string"
              ? answer.content
              : JSON.stringify(answer.content, null, 2);
        const type = format === "markdown" ? "text/markdown" : answer.encoding === "base64" ? "application/zip" : "application/json";
        const url = URL.createObjectURL(new Blob([body], { type }));
        const link = document.createElement("a");
        link.href = url;
        link.download = answer.file;
        link.click();
        setTimeout(() => URL.revokeObjectURL(url), 1000);
        close(true);
      },
      (error) => t("dialog.exportFailed", { error }),
    );
  return (
    <>
      <DialogBody>
        <div className="field">
          <label className="check">
            <input type="radio" name="export-format" data-testid="export-markdown" checked={format === "markdown"} onChange={() => setFormat("markdown")} />
            {t("dialog.exportMarkdown")}
          </label>
          <label className="check">
            <input type="radio" name="export-format" data-testid="export-archive" checked={format === "archive"} onChange={() => setFormat("archive")} />
            {t("dialog.exportArchive")}
          </label>
          <label className="check">
            <input
              type="checkbox"
              data-testid="export-recording"
              disabled={format !== "archive"}
              checked={format === "archive" && recording}
              onChange={(e) => setRecording(e.target.checked)}
            />
            {t("dialog.exportRecording")}
          </label>
          <div className="hint">{t("dialog.exportRecordingHint")}</div>
        </div>
      </DialogBody>
      <DialogFoot>
        <button type="button" className="btn outline" onClick={() => close(null)}>
          {t("common.cancel")}
        </button>
        <button type="button" className="btn primary" data-testid="export-go" onClick={() => void go()}>
          {t("dialog.exportGo")}
        </button>
      </DialogFoot>
    </>
  );
}

/* ── import ───────────────────────────────────────────────────────────── */

/** What `harnix-app.sessions.import` answers: the engine's, plus where it landed. */
interface Imported {
  root_session_id: string;
  instance?: string;
  warnings?: string[];
}

type ImportState = { step: "idle" } | { step: "uploading" | "importing"; name: string } | { step: "failed"; error: string };

/**
 * Import a session bundle (an archive exported from here or from Agix): the zip goes up the
 * large-payload channel, then the backend asks the engine to install it as new sessions in the
 * session space. The bundle's project is kept when it exists on this machine, else the session
 * has none (a warning says so). The imported session is opened.
 */
export async function importDialog(host: HostApi, store: AppStore): Promise<void> {
  await host.dialogs.open<true>((close) => <ImportForm host={host} store={store} close={close} />, { title: t("dialog.importSession") });
}

function ImportForm({ host, store, close }: { host: HostApi; store: AppStore; close: Close }) {
  const t = useT();
  const pick = useRef<HTMLInputElement>(null);
  const [state, setState] = useState<ImportState>({ step: "idle" });
  const busy = state.step === "uploading" || state.step === "importing";
  const run = async (file: File) => {
    setState({ step: "uploading", name: file.name });
    try {
      const grant = await host.call<{ token: string; url: string }>("harnix-app.sessions.importUpload", { name: file.name, space: SESSIONS });
      const response = await fetch(grant.url, { method: "POST", body: file });
      if (!response.ok) throw new Error(await response.text());
      setState({ step: "importing", name: file.name });
      const done = await host.call<Imported>("harnix-app.sessions.import", { token: grant.token, space: SESSIONS });
      await store.refresh();
      for (const warning of done.warnings ?? []) host.shell.notice(warning);
      close(true);
      await host.session.open({ instance: done.instance ?? host.defaults.instance, sessionId: done.root_session_id });
    } catch (e) {
      setState({ step: "failed", error: errorText(e) });
    }
  };
  return (
    <>
      <DialogBody>
        <div className="field">
          <div className="hint">{t("dialog.importHint")}</div>
          <input
            ref={pick}
            type="file"
            accept=".zip"
            hidden
            data-testid="import-file"
            onChange={(e) => {
              const file = e.target.files?.[0];
              e.target.value = "";
              if (file) void run(file);
            }}
          />
          <button type="button" className="btn outline" data-testid="import-pick" disabled={busy} onClick={() => pick.current?.click()}>
            {t("dialog.importPick")}
          </button>
          <div className="hint" data-testid="import-status" role="status">
            {state.step === "uploading" ? t("dialog.importUploading", { name: state.name }) : null}
            {state.step === "importing" ? t("dialog.importImporting", { name: state.name }) : null}
            {state.step === "failed" ? t("dialog.importFailed", { error: state.error }) : null}
          </div>
        </div>
      </DialogBody>
      <DialogFoot>
        <button type="button" className="btn outline" disabled={busy} onClick={() => close(null)}>
          {t("common.cancel")}
        </button>
      </DialogFoot>
    </>
  );
}

/* ── trusting a project ───────────────────────────────────────────────── */

/** A kind of setting that waits on trust, in words. */
export const trustCategory = (category: string) => t(`projects.trust.cat.${category}`);

/** The kinds a status lists, in words, joined. */
export const trustCategories = (status: TrustStatus) => status.categories.map(trustCategory).join(t("projects.trust.sep"));

/**
 * Trust a project: what it lets in is read again first and listed by kind, each with what it names
 * (`harnix-app.projects.status`), and the project is trusted only once that is confirmed. A
 * project already trusted, or one with no settings of its own, is said so and nothing is asked.
 */
export async function trustDialog(host: HostApi, store: AppStore, root: string, name: string): Promise<void> {
  let status: TrustStatus | null;
  try {
    status = await store.loadTrust(root);
  } catch (e) {
    host.shell.notice(t("projects.trust.failed", { error: errorText(e) }), "error");
    return;
  }
  if (!status) return;
  if (status.trust === "trusted") return host.shell.notice(t("projects.trust.already", { name }));
  if (status.trust === "none") return host.shell.notice(t("projects.trust.none", { name }));
  const read = status;
  await host.dialogs.open<true>((close) => <TrustForm host={host} store={store} root={root} name={name} status={read} close={close} />, {
    title: t("projects.trust.title"),
  });
}

function TrustForm({ host, store, root, name, status, close }: { host: HostApi; store: AppStore; root: string; name: string; status: TrustStatus; close: Close }) {
  const t = useT();
  const byKind = (category: string) => status.items.filter((item) => item.category === category);
  const named = (items: TrustItem[]) => items.map((item) => item.name ?? item.key).join(t("projects.trust.sep"));
  const trust = () =>
    attempt(
      host,
      async () => {
        await store.setTrust(root, true);
        host.shell.notice(t("projects.trust.done", { name }));
        close(true);
      },
      (error) => t("projects.trust.failed", { error }),
    );
  return (
    <>
      <DialogBody>
        <p data-testid="trust-intro">{t("projects.trust.intro", { path: root })}</p>
        {status.categories.length ? (
          <>
            <p>{t("projects.trust.enables")}</p>
            <ul className="trust-list" data-testid="trust-categories">
              {status.categories.map((category) => (
                <li key={category} data-category={category}>
                  <strong>{trustCategory(category)}</strong>
                  <span className="sub">{named(byKind(category))}</span>
                </li>
              ))}
            </ul>
          </>
        ) : (
          <p data-testid="trust-nothing">{t("projects.trust.nothing")}</p>
        )}
        <div className="hint">{t("projects.trust.warning")}</div>
      </DialogBody>
      <DialogFoot>
        <button type="button" className="btn outline" onClick={() => close(null)}>
          {t("common.cancel")}
        </button>
        <button type="button" className="btn primary" data-testid="trust-confirm" onClick={() => void trust()}>
          {t("projects.trust.confirm")}
        </button>
      </DialogFoot>
    </>
  );
}

/** No longer trust a project: tightening needs no confirmation. A glob or the default in the global
 *  settings may still trust it, which is said. */
export async function untrust(host: HostApi, store: AppStore, root: string, name: string): Promise<void> {
  await attempt(
    host,
    async () => {
      const status = await store.setTrust(root, false);
      if (status.trust !== "trusted") host.shell.notice(t("projects.trust.revoked", { name }));
      else if (status.matched) host.shell.notice(t("projects.trust.stillTrusted", { name, matched: status.matched }));
      else host.shell.notice(t("projects.trust.stillTrustedDefault", { name }));
    },
    (error) => t("projects.trust.failed", { error }),
  );
}

/* ── a project's menu ─────────────────────────────────────────────────── */

/** A project's menu, anchored where it was opened; `more` goes after "new session" (what the
 *  registered item kinds make: a task). */
/**
 * A project's trust as the kinds of item see it (`ProjectItemContext.trust`): read again each
 * time, and asked for with the same dialog as the project's menu. A project whose trust cannot be
 * read says null.
 */
export function projectTrust(host: HostApi, store: AppStore, workspace: Workspace): ProjectTrust {
  const root = workspace.path;
  const name = workspace.title ?? workspace.path;
  const status = async () => {
    try {
      return (await store.loadTrust(root))?.trust ?? null;
    } catch {
      return null;
    }
  };
  return {
    status,
    ask: async () => {
      await trustDialog(host, store, root, name);
      return store.getSnapshot().trust[root]?.trust === "trusted";
    },
  };
}

/** What a kind of item is handed about a project: its directory, the session space, its trust. */
export function projectContext(host: HostApi, store: AppStore, workspace: Workspace): ProjectItemContext {
  return { root: workspace.path, workspace, space: SESSIONS, trust: projectTrust(host, store, workspace) };
}

export function workspaceMenu(host: HostApi, store: AppStore, workspace: Workspace, anchor: Element | { x: number; y: number }, more: MenuItem[] = []): void {
  const name = workspace.title ?? workspace.path;
  // Trust is offered for a project whose trust could be read, or is still being read: not for one
  // whose read failed (the backend was not granted it), nor a directory with no settings of its own.
  const status = store.getSnapshot().trust[workspace.path];
  const trust =
    status === null || status?.trust === "none"
      ? []
      : status?.trust === "trusted"
        ? [{ id: "untrust", icon: "shield", label: t("projects.trust.revoke"), run: () => untrust(host, store, workspace.path, name) }]
        : [{ id: "trust", icon: "shield", label: t("projects.trust.menu"), run: () => trustDialog(host, store, workspace.path, name) }];
  void host.dialogs.menu(anchor, [
    { id: "new", icon: "plus", label: t("dialog.newSessionHere"), run: () => newSessionDialog(host, store, workspace.path) },
    ...more,
    ...trust,
    {
      id: "rename",
      icon: "edit",
      label: t("common.rename"),
      run: async () => {
        const title = await host.dialogs.prompt({ title: t("dialog.renameProject"), label: t("common.name"), value: workspace.title ?? "" });
        if (title === null) return;
        if (await attempt(host, () => host.call("harnix-app.workspaces.update", { id: workspace.id, title }))) void store.refresh();
      },
    },
    {
      id: "remove",
      icon: "trash",
      danger: true,
      label: t("dialog.removeGroup"),
      run: async () => {
        if (await attempt(host, () => host.call("harnix-app.workspaces.remove", { id: workspace.id }))) void store.refresh();
      },
    },
  ]);
}
