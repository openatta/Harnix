/**
 * The modal dialogs: new session, add project, rename, the session and workspace menus, confirm,
 * and the directory picker they share. Each opens through `App.openModal` and renders its own
 * `.body` and `.foot`, so a dialog's buttons can read the dialog's state.
 */

import { useEffect, useRef, useState, type ReactNode } from "react";
import { errorText, type Scene } from "@harnix/core";
import { t, useT } from "@harnix/react";

import type { App, ShellSession, Workspace } from "./app.ts";
import { Glyph } from "./glyph.tsx";

export function Body({ children }: { children: ReactNode }) {
  return <div className="body">{children}</div>;
}

export function Foot({ children }: { children?: ReactNode }) {
  return <div className="foot">{children}</div>;
}

export function CloseFoot({ app }: { app: App }) {
  const t = useT();
  return (
    <Foot>
      <button type="button" className="btn" onClick={() => app.closeModal()}>
        {t("common.close")}
      </button>
    </Foot>
  );
}

/** A modal with nothing to decide: the body and a close button. */
export function info(app: App, title: string, body: ReactNode, wide = false): void {
  app.openModal({
    title,
    wide,
    content: (
      <>
        <Body>{body}</Body>
        <CloseFoot app={app} />
      </>
    ),
  });
}

async function attempt(app: App, run: () => Promise<unknown>, failed?: (error: string) => string): Promise<boolean> {
  try {
    await run();
    return true;
  } catch (e) {
    app.notice(failed ? failed(errorText(e)) : errorText(e), "error");
    return false;
  }
}

/* ── directory picker ─────────────────────────────────────────────────── */

interface Listing {
  path: string;
  breadcrumbs: { name: string; path: string }[];
  entries: { name: string; hidden?: boolean }[];
}

export function DirectoryPicker({ app, onPick }: { app: App; onPick: (path: string) => void }) {
  const t = useT();
  const [listing, setListing] = useState<Listing | null>(null);
  const open = async (path?: string) => {
    await attempt(app, async () => setListing(await app.call<Listing>("harnix.listDirectory", path ? { path } : {})));
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
        <button type="button" className="btn sm outline" onClick={() => onPick(listing.path)}>
          {t("dialog.useThisDirectory")}
        </button>
      </div>
      <div className="list">
        {entries.map((entry) => (
          <button
            type="button"
            key={entry.name}
            className="entry"
            onClick={() => void open(`${listing.path.replace(/\/$/, "")}/${entry.name}`)}
          >
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

export async function newSessionDialog(app: App, preselect?: string): Promise<void> {
  let recents: Recents = {};
  try {
    recents = await app.call<Recents>("harnix.recentProjects", {});
  } catch {
    /* the picker still works without recents */
  }
  app.openModal({
    title: t("dialog.newSession"),
    content: <NewSession app={app} recents={recents} preselect={preselect} />,
  });
}

function NewSession({ app, recents, preselect }: { app: App; recents: Recents; preselect?: string }) {
  const t = useT();
  const scenes = (app.snapshot.hello?.scenes ?? []) as (Scene & { capabilities?: { requires_project?: boolean } })[];
  const active = scenes.filter((scene) => scene.active);
  const [scene, setScene] = useState(active[0]?.scene ?? "coding");
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
      app,
      async () => {
        await app.create({ scene, project_root: root, options: { permission_mode: mode } });
        app.closeModal();
      },
      (error) => t("dialog.createFailed", { error }),
    );

  return (
    <>
      <Body>
        <div className="field">
          <label>{t("dialog.scene")}</label>
          <select data-testid="new-scene" value={scene} onChange={(e) => setScene(e.target.value)}>
            {scenes.map((s) => (
              <option key={s.scene} value={s.scene}>
                {[
                  s.name || s.scene,
                  (s as { capabilities?: { requires_project?: boolean } }).capabilities?.requires_project ? t("dialog.sceneNeedsProject") : "",
                  s.active ? "" : t("dialog.sceneInactive"),
                ].join("")}
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
          <DirectoryPicker app={app} onPick={setRoot} />
          <div className="picked" data-testid="new-root">
            {root ?? t("dialog.noProject")}
          </div>
        </div>
      </Body>
      <Foot>
        <button type="button" className="btn outline" onClick={() => app.closeModal()}>
          {t("common.cancel")}
        </button>
        <button type="button" className="btn primary" data-testid="new-create" onClick={() => void create()}>
          {t("common.create")}
        </button>
      </Foot>
    </>
  );
}

/* ── add project ──────────────────────────────────────────────────────── */

export function addWorkspaceDialog(app: App): void {
  app.openModal({ title: t("dialog.addProject"), content: <AddWorkspace app={app} /> });
}

function AddWorkspace({ app }: { app: App }) {
  const t = useT();
  const [name, setName] = useState("");
  const [chosen, setChosen] = useState<string | null>(null);
  const add = async () => {
    if (!name.trim() && !chosen) {
      app.notice(t("dialog.pickDirectoryFirst"));
      return;
    }
    const ok = await attempt(
      app,
      () =>
        name.trim()
          ? app.call("harnix.projects.create", { name: name.trim() })
          : app.call("harnix.workspaces.create", { path: chosen }),
      (error) => t("dialog.addFailed", { error }),
    );
    if (ok) {
      app.closeModal();
      void app.refreshSessions();
    }
  };
  return (
    <>
      <Body>
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
          <DirectoryPicker app={app} onPick={setChosen} />
          <div className="picked">{chosen ?? t("dialog.noDirectoryChosen")}</div>
        </div>
      </Body>
      <Foot>
        <button type="button" className="btn outline" onClick={() => app.closeModal()}>
          {t("common.cancel")}
        </button>
        <button type="button" className="btn primary" onClick={() => void add()}>
          {t("common.add")}
        </button>
      </Foot>
    </>
  );
}

/* ── rename, confirm ──────────────────────────────────────────────────── */

export function renameDialog(app: App, title: string, current: string | null | undefined, save: (title: string) => Promise<unknown>): void {
  app.openModal({ title, content: <Rename app={app} current={current ?? ""} save={save} /> });
}

function Rename({ app, current, save }: { app: App; current: string; save: (title: string) => Promise<unknown> }) {
  const t = useT();
  const [value, setValue] = useState(current);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus(), []);
  const commit = async () => {
    if (await attempt(app, () => save(value))) {
      app.closeModal();
      void app.refreshSessions();
    }
  };
  return (
    <>
      <Body>
        <div className="field">
          <label>{t("common.name")}</label>
          <input
            ref={input}
            type="text"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void commit();
            }}
          />
        </div>
      </Body>
      <Foot>
        <button type="button" className="btn outline" onClick={() => app.closeModal()}>
          {t("common.cancel")}
        </button>
        <button type="button" className="btn primary" onClick={() => void commit()}>
          {t("common.save")}
        </button>
      </Foot>
    </>
  );
}

export function confirmDialog(app: App, message: string, confirm: () => Promise<unknown>): void {
  app.openModal({
    title: t("dialog.confirm"),
    content: (
      <>
        <Body>
          <div>{message}</div>
        </Body>
        <Foot>
          <button type="button" className="btn outline" onClick={() => app.closeModal()}>
            {t("common.cancel")}
          </button>
          <button type="button" className="btn primary" data-testid="confirm" onClick={() => void attempt(app, confirm)}>
            {t("common.confirm")}
          </button>
        </Foot>
      </>
    ),
  });
}

/* ── menus ────────────────────────────────────────────────────────────── */

function MenuItem({ icon, label, sub, danger, onClick }: { icon: string; label: string; sub?: string; danger?: boolean; onClick: () => void }) {
  return (
    <button type="button" className={danger ? "danger" : ""} onClick={onClick}>
      <Glyph name={icon} className="glyph" />
      <span>
        {label}
        {sub ? <span className="sub">{sub}</span> : null}
      </span>
    </button>
  );
}

export function sessionMenu(app: App, session: ShellSession): void {
  const key = { instance: session.instance ?? "system", session_id: session.session_id, space: app.snapshot.space };
  const reopen = (sessionId: string) => app.open({ instance: key.instance, sessionId });
  app.openModal({
    title: session.name || t("sidebar.untitled"),
    content: (
      <>
        <Body>
          <div className="picked">{session.session_id}</div>
          <div className="menu-list">
            <MenuItem
              icon="chat"
              label={t("common.open")}
              onClick={() => {
                app.closeModal();
                reopen(session.session_id);
              }}
            />
            <MenuItem
              icon="edit"
              label={t("common.rename")}
              onClick={() => renameDialog(app, t("dialog.renameSession"), session.name, (title) => app.call("harnix.sessions.rename", { ...key, title }))}
            />
            <MenuItem
              icon="archive"
              label={session.archived ? t("dialog.unarchive") : t("dialog.archive")}
              sub={t("dialog.archiveHint")}
              onClick={async () => {
                if (await attempt(app, () => app.call("harnix.sessions.archive", { ...key, archived: !session.archived }))) {
                  app.closeModal();
                  void app.refreshSessions();
                }
              }}
            />
            <MenuItem
              icon="fork"
              label={t("dialog.fork")}
              onClick={() =>
                void attempt(
                  app,
                  async () => {
                    const forked = await app.call<{ session_id: string }>("session.fork", key);
                    app.closeModal();
                    await app.refreshSessions();
                    reopen(forked.session_id);
                  },
                  (error) => t("dialog.forkFailed", { error }),
                )
              }
            />
            <MenuItem
              icon="archive"
              label={t("dialog.closeSession")}
              sub={t("dialog.closeSessionHint")}
              onClick={() =>
                void attempt(
                  app,
                  async () => {
                    await app.call("session.close", key);
                    app.closeModal();
                    void app.refreshSessions();
                  },
                  (error) => t("dialog.closeFailed", { error }),
                )
              }
            />
            <MenuItem
              icon="trash"
              danger
              label={t("dialog.deleteSession")}
              sub={t("dialog.deleteSessionHint")}
              onClick={() =>
                void attempt(
                  app,
                  async () => {
                    const plan = await app.call<{ sidechains_deleted?: number }>("session.delete", { ...key, dry_run: true });
                    confirmDialog(app, t("dialog.deleteConfirm", { count: plan.sidechains_deleted ?? 0 }), async () => {
                      await app.call("session.delete", key);
                      app.closeModal();
                      app.forget({ instance: key.instance, sessionId: key.session_id });
                      void app.refreshSessions();
                    });
                  },
                  (error) => t("dialog.deleteFailed", { error }),
                )
              }
            />
          </div>
        </Body>
        <CloseFoot app={app} />
      </>
    ),
  });
}

export function workspaceMenu(app: App, workspace: Workspace): void {
  app.openModal({
    title: workspace.title ?? workspace.path,
    content: (
      <>
        <Body>
          <div className="picked">{workspace.path || t("dialog.noProject")}</div>
          <div className="menu-list">
            <MenuItem
              icon="plus"
              label={t("dialog.newSessionHere")}
              onClick={() => {
                app.closeModal();
                void newSessionDialog(app, workspace.path);
              }}
            />
            <MenuItem
              icon="edit"
              label={t("common.rename")}
              onClick={() =>
                renameDialog(app, t("dialog.renameProject"), workspace.title, (title) =>
                  app.call("harnix.workspaces.update", { id: workspace.id, title }),
                )
              }
            />
            <MenuItem
              icon="trash"
              danger
              label={t("dialog.removeGroup")}
              sub={t("dialog.removeGroupHint")}
              onClick={async () => {
                if (await attempt(app, () => app.call("harnix.workspaces.remove", { id: workspace.id }))) {
                  app.closeModal();
                  void app.refreshSessions();
                }
              }}
            />
          </div>
        </Body>
        <CloseFoot app={app} />
      </>
    ),
  });
}

/* ── the connection ───────────────────────────────────────────────────── */

/** One connection to this process, as `harnix.process` lists it. */
export interface Connection {
  client: number;
  device: string;
  topology: string;
  user_agent?: string;
  since_ms?: number;
}

/** What kind of client a user agent is, roughly: a desktop shell, a phone, a browser. */
export function clientKind(agent: string | undefined): "desktop" | "mobile" | "browser" | "script" {
  const ua = agent ?? "";
  if (!ua) return "script";
  if (/Tauri|Electron|harnix-desktop/i.test(ua)) return "desktop";
  if (/Mobile|Android|iPhone|iPad/i.test(ua)) return "mobile";
  if (/Mozilla|WebKit|Chrome|Safari|Firefox/i.test(ua)) return "browser";
  return "script";
}

/** The users connected right now: device, kind, how, since when. */
export function Connections({ connections }: { connections: Connection[] }) {
  const t = useT();
  if (!connections.length) return <div className="hint">{t("process.noConnections")}</div>;
  return (
    <div className="connections" data-testid="connections">
      {connections.map((c) => (
        <div key={c.client} className="conn-row" data-device={c.device}>
          <span className="dev">{c.device}</span>
          <span className="kind">{t(`process.kind.${clientKind(c.user_agent)}`)}</span>
          <span className="topo">{c.topology}</span>
          <span className="since">{c.since_ms ? since(c.since_ms) : ""}</span>
        </div>
      ))}
    </div>
  );
}

export function since(ms: number): string {
  const secs = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m`;
  return `${Math.floor(secs / 3600)}h${String(Math.floor((secs % 3600) / 60)).padStart(2, "0")}m`;
}

/** This page's connection, and everyone else's. */
export function connectionDialog(app: App): void {
  info(app, t("avatar.connection"), <ConnectionBody app={app} />);
}

function ConnectionBody({ app }: { app: App }) {
  const t = useT();
  const s = app.snapshot;
  const [connections, setConnections] = useState<Connection[] | null>(null);
  useEffect(() => {
    void app
      .call<{ connections?: Connection[] }>("harnix.process", {})
      .then((r) => setConnections(r.connections ?? []))
      .catch(() => setConnections([]));
  }, [app]);
  return (
    <div className="connection" data-testid="connection-details">
      <dl className="facts">
        <div className="fact">
          <dt>{t("avatar.state")}</dt>
          <dd>{s.connection}</dd>
        </div>
        <div className="fact">
          <dt>{t("avatar.topology")}</dt>
          <dd>{app.client.topology ?? ""}</dd>
        </div>
        <div className="fact">
          <dt>{t("avatar.address")}</dt>
          <dd>{location.host}</dd>
        </div>
        <div className="fact">
          <dt>{t("avatar.protocol")}</dt>
          <dd>v{s.hello?.protocol_version ?? "?"}</dd>
        </div>
      </dl>
      <div className="label">{t("process.users")}</div>
      {connections ? <Connections connections={connections} /> : null}
    </div>
  );
}
