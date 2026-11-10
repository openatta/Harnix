/**
 * The shell's own dialogs: rename, confirm, the connection; and the session menu, which opens
 * them. Each dialog opens through `App.openModal` and renders its own `.body` and `.foot`, so a
 * dialog's buttons can read the dialog's state. What a UI package asks for goes through
 * `host.dialogs` (`host-dialogs.tsx`).
 */

import { useEffect, useRef, useState, type ReactNode } from "react";
import { errorText } from "@harnix/core";
import { DialogBody, DialogFoot, t, useT } from "@harnix/react";

import type { App } from "./app.ts";
import { actionMenuItems } from "./actions.tsx";
import { openMenu, type MenuEntry } from "./Menu.tsx";

export const Body = DialogBody;
export const Foot = DialogFoot;

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
        <button type="button" className="btn primary" data-testid="rename-save" onClick={() => void commit()}>
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
          <button type="button" className="btn outline" data-testid="confirm-cancel" onClick={() => app.closeModal()}>
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

/** Where a menu was opened from: the element the focus goes back to, and the point it opens at. */
export interface MenuOrigin {
  origin: HTMLElement | null;
  at: { x: number; y: number };
}

/** The session a menu is about: where it is, and the name it shows. */
export interface MenuSession {
  instance: string;
  session_id: string;
  name?: string | null;
}

/**
 * A session's menu, anchored where it was opened (its row, `⋯`, the title). Only what has to be
 * confirmed opens a dialog. The shell's own items use the kernel's methods; what an extension
 * offers on a session (archiving, exporting, opening it elsewhere) comes from `session.action`.
 */
export function sessionMenu(app: App, session: MenuSession, from: MenuOrigin): void {
  // The backend space the front space lists, not its id: the projects space lists the session
  // space's sessions.
  const key = { instance: session.instance, session_id: session.session_id, space: app.claim() };
  const reopen = (sessionId: string) => app.open({ instance: key.instance, sessionId });
  const items: MenuEntry[] = [
    { id: "open", icon: "chat", label: t("common.open"), run: () => reopen(session.session_id) },
    {
      id: "rename",
      icon: "edit",
      label: t("common.rename"),
      run: () => renameDialog(app, t("dialog.renameSession"), session.name, (title) => app.call("harnix.session.rename", { ...key, title })),
    },
    {
      id: "fork",
      icon: "fork",
      label: t("dialog.fork"),
      run: () =>
        void attempt(
          app,
          async () => {
            const forked = await app.call<{ session_id: string }>("harnix.session.fork", key);
            await app.refreshSessions();
            reopen(forked.session_id);
          },
          (error) => t("dialog.forkFailed", { error }),
        ),
    },
    {
      id: "close",
      icon: "archive",
      label: t("dialog.closeSession"),
      sub: t("dialog.closeSessionHint"),
      run: () =>
        void attempt(
          app,
          async () => {
            await app.call("harnix.session.close", key);
            void app.refreshSessions();
          },
          (error) => t("dialog.closeFailed", { error }),
        ),
    },
    {
      id: "delete",
      icon: "trash",
      danger: true,
      label: t("dialog.deleteSession"),
      sub: t("dialog.deleteSessionHint"),
      run: () =>
        void attempt(
          app,
          async () => {
            const plan = await app.call<{ sidechains_deleted?: number }>("harnix.session.delete", { ...key, dry_run: true });
            confirmDialog(app, t("dialog.deleteConfirm", { count: plan.sidechains_deleted ?? 0 }), async () => {
              await app.call("harnix.session.delete", key);
              app.closeModal();
              app.forget({ instance: key.instance, sessionId: key.session_id });
              void app.refreshSessions();
            });
          },
          (error) => t("dialog.deleteFailed", { error }),
        ),
    },
  ];
  // What extensions offer on a session, after the shell's own and before deleting it.
  const target = { kind: "session" as const, session: { instance: key.instance, sessionId: key.session_id }, space: app.snapshot.space };
  items.splice(items.length - 1, 0, ...actionMenuItems(app, target));
  const name = session.name || t("sidebar.untitled");
  openMenu({ label: t("sidebar.sessionActions"), title: name, items, at: from.at, returnTo: from.origin, testId: "session-menu", className: "session-menu" });
}

/* ── the connection ───────────────────────────────────────────────────── */

/** This page's connection: its state, how it is carried, to where, at which protocol. */
export function connectionDialog(app: App): void {
  info(app, t("avatar.connection"), <ConnectionBody app={app} />);
}

function ConnectionBody({ app }: { app: App }) {
  const t = useT();
  const s = app.snapshot;
  return (
    <div className="connection" data-testid="connection-details">
      <dl className="facts">
        <div className="fact">
          <dt>{t("avatar.state")}</dt>
          <dd>{s.connection}</dd>
        </div>
        <div className="fact">
          <dt>{t("avatar.topology")}</dt>
          <dd>{app.conn.topology ?? ""}</dd>
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
    </div>
  );
}
