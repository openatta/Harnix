/**
 * The frame: banner, three columns (sessions · conversation · details) and the modal host.
 *
 * The column ids, `#frame`'s data attributes and the class names are the old interface's, so the
 * layout stylesheet carried over unchanged and a reader of either finds the same landmarks. What is
 * inside the conversation column is `ChatPanel`, the conversation trunk shared with IPDrun (D21).
 */

import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { errorText } from "@harnix/core";
import type { SessionController, SessionState } from "@harnix/core/state";
import { ChatPanel, Contributed, t, useRegistry, useT, type Attached, type ChatCommand, type ComposerApi, type MentionItem } from "@harnix/react";

import { keyOf, type App, type ShellState } from "./app.ts";
import { followEnvelopes } from "./envelope.tsx";
import { Glyph } from "./glyph.tsx";
import { newSessionDialog } from "./dialogs.tsx";
import { Sidebar, spacesOf } from "./Sidebar.tsx";

export function Shell({ app }: { app: App }) {
  const s = useSyncExternalStore(app.subscribe, app.getSnapshot);
  useTheme(s.theme);
  useShortcuts(app);
  return (
    <>
      <Banner app={app} s={s} />
      <div id="frame" data-sidebar={s.sidebar} data-details={s.detailsOpen ? "open" : "closed"}>
        <Sidebar app={app} s={s} />
        <Main app={app} s={s} />
        <Details app={app} s={s} />
      </div>
      <ModalHost app={app} s={s} />
    </>
  );
}

/* ── theme and keys ───────────────────────────────────────────────────── */

function systemDark(): boolean {
  return typeof matchMedia === "function" && matchMedia("(prefers-color-scheme: dark)").matches;
}

function useTheme(choice: ShellState["theme"]) {
  const [dark, setDark] = useState(systemDark);
  useEffect(() => {
    if (typeof matchMedia !== "function") return;
    const query = matchMedia("(prefers-color-scheme: dark)");
    const change = () => setDark(query.matches);
    query.addEventListener("change", change);
    return () => query.removeEventListener("change", change);
  }, []);
  const theme = choice === "system" ? (dark ? "dark" : "light") : choice;
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    const meta = document.querySelector('meta[name="theme-color"]');
    if (meta) meta.setAttribute("content", getComputedStyle(document.body).backgroundColor);
  }, [theme]);
}

export function isDark(): boolean {
  return document.documentElement.dataset.theme === "dark";
}

function useShortcuts(app: App) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const mod = event.metaKey || event.ctrlKey;
      if (event.key === "Escape" && app.snapshot.modal) {
        app.closeModal();
      } else if (mod && event.key.toLowerCase() === "k") {
        event.preventDefault();
        void newSessionDialog(app);
      } else if (mod && event.key === "\\") {
        event.preventDefault();
        app.toggleSidebar();
      } else if (mod && event.key === "/") {
        event.preventDefault();
        document.querySelector<HTMLTextAreaElement>('[data-testid="conv-input"]')?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [app]);
}

/* ── banner ───────────────────────────────────────────────────────────── */

function Banner({ app, s }: { app: App; s: ShellState }) {
  const t = useT();
  return (
    <div id="banner" className={s.banner ? "on" : ""} data-tone={s.banner?.tone}>
      {s.banner ? (
        <>
          <Glyph name="alert" className="glyph" />
          <span className="text">{s.banner.text}</span>
          <button type="button" className="icon-btn close" title={t("banner.dismiss")} onClick={() => app.dismissBanner()}>
            <Glyph name="close" className="glyph" />
          </button>
        </>
      ) : null}
    </div>
  );
}

/* ── the conversation column ──────────────────────────────────────────── */

function Main({ app, s }: { app: App; s: ShellState }) {
  const registry = useRegistry(app.registry);
  const [controller, setController] = useState<SessionController | null>(null);
  const session = useControllerState(controller);
  const space = spacesOf(app).find((c) => c.id === s.space);
  const center = space?.center ?? "chat";
  const [expanded, setExpanded] = useState(false);
  if (center !== "chat" && space) {
    return (
      <section id="main" className={`overview${expanded ? " expanded" : ""}`} data-space={s.space}>
        <div id="overview" data-testid="overview">
          <Contributed registry={registry} point="space" contribution={space}>
            {space.overview?.() ?? null}
          </Contributed>
        </div>
        {center === "minimized" ? (
          <div className="chat-bar" data-testid="chat-bar" data-state={expanded ? "open" : "closed"}>
            <button type="button" className="icon-btn bar-toggle" data-testid="chat-bar-toggle" title={t("app.details")} onClick={() => setExpanded((v) => !v)}>
              <Glyph name={expanded ? "chevronDown" : "chevronRight"} className="glyph" />
            </button>
            {s.current ? (
              <Conversation key={keyOf(s.current.instance, s.current.sessionId)} app={app} s={s} onController={setController} />
            ) : (
              <FirstMessage app={app} onSent={() => setExpanded(true)} />
            )}
          </div>
        ) : null}
      </section>
    );
  }
  const empty = !s.current || !session || (!session.loading && session.blocks.length === 0);
  return (
    <section id="main" className={empty ? "empty" : ""} data-space={s.space}>
      <Header app={app} s={s} session={session} />
      {s.current ? (
        <Conversation key={keyOf(s.current.instance, s.current.sessionId)} app={app} s={s} onController={setController} />
      ) : (
        <Hero />
      )}
    </section>
  );
}

/** A space's bar with no session yet: the first message starts one, in this space. */
function FirstMessage({ app, onSent }: { app: App; onSent: () => void }) {
  const t = useT();
  const [text, setText] = useState("");
  const send = async () => {
    const message = text.trim();
    if (!message) return;
    try {
      const key = await app.create({});
      await app.sessionCall("harnix.send", { instance: key.instance, session_id: key.sessionId }, { message });
      setText("");
      onSent();
    } catch (e) {
      app.notice(errorText(e), "error");
    }
  };
  return (
    <div className="hx-cmp first-message">
      <div className="hx-cmp-box">
        <textarea
          className="hx-cmp-ta"
          data-testid="conv-input"
          rows={1}
          value={text}
          placeholder={t("composer.placeholder")}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void send();
            }
          }}
        />
      </div>
    </div>
  );
}

const NOTHING = () => () => {};

function useControllerState(controller: SessionController | null): SessionState | null {
  const subscribe = controller ? controller.subscribe : NOTHING;
  return useSyncExternalStore(subscribe, () => (controller ? controller.getSnapshot() : null));
}

function Hero() {
  const t = useT();
  return (
    <div id="scroll">
      <div id="flow">
        <div id="hero">
          <Glyph name="brand" className="mark" />
          <h1>{t("hero.title")}</h1>
          <p>{t("hero.body")}</p>
        </div>
      </div>
    </div>
  );
}

/* What the turn spent: the four usage figures do not overlap, so the total is all four, and the
 * cache pill names the part of that total the cache served. */
function usagePills(t: (k: string, v?: Record<string, unknown>) => string, usage: SessionState["usage"]) {
  if (!usage) return null;
  const n = (v: number | undefined) => v ?? 0;
  const cached = n(usage.cache_read_input_tokens) + n(usage.cache_creation_input_tokens);
  const count = n(usage.input_tokens) + n(usage.output_tokens) + cached;
  return (
    <>
      <span className="pill" data-testid="head-tokens">{t("header.tokens", { count })}</span>
      {cached ? <span className="pill">{t("header.cached", { count: cached })}</span> : null}
    </>
  );
}

function Header({ app, s, session }: { app: App; s: ShellState; session: SessionState | null }) {
  const t = useT();
  const row = s.current ? s.sessions.find((r) => r.session_id === s.current!.sessionId && r.instance === s.current!.instance) : null;
  const name = row?.name ?? session?.session?.name ?? null;
  const scene = session?.session?.scene ?? row?.scene;
  const conn = s.connection === "open" ? "on" : s.connection === "connecting" || s.connection === "degraded" ? "" : "off";
  const label =
    s.connection === "open"
      ? t("conn.open")
      : s.connection === "connecting" || s.connection === "degraded"
        ? t("conn.connecting")
        : s.connection === "incompatible"
          ? t("conn.incompatible")
          : t("conn.closed");
  return (
    <header id="head">
      <div className="title truncate">{s.current ? name || t("sidebar.untitled") : t("app.title")}</div>
      {s.current ? (
        <div className="meta">
          {scene ? <span className="pill">{scene}</span> : null}
          {usagePills(t, session?.usage ?? null)}
        </div>
      ) : null}
      <div className="spacer" />
      <div id="conn" className={conn} data-state={s.connection}>
        <span className="dot" />
        <span className="label">{label}</span>
      </div>
      <button type="button" className="icon-btn" data-testid="details-toggle" title={t("app.details")} onClick={() => app.toggleDetails()}>
        <Glyph name="details" className="glyph" />
      </button>
    </header>
  );
}

function Conversation({ app, s, onController }: { app: App; s: ShellState; onController: (c: SessionController | null) => void }) {
  const t = useT();
  const registry = useRegistry(app.registry);
  const current = s.current!;
  const [controller, setController] = useState<SessionController | null>(null);
  const bind = useCallback(
    (c: SessionController | null) => {
      app.controller = c;
      setController(c);
      onController(c);
    },
    [app, onController],
  );
  const bindComposer = useCallback((api: ComposerApi | null) => {
    app.composer = api;
  }, [app]);
  useEffect(() => (controller ? followEnvelopes(app, controller) : undefined), [app, controller]);

  const commands = useCommands(app);
  const projectRoot = s.sessions.find((r) => r.session_id === current.sessionId)?.project_root ?? null;
  const filesAsked = useRef(0);
  // `@` offers the project's files and — after `@session:` — sessions of any space, which go to
  // the turn as a copy of what was said in them (design §8.3: sessions do not cross spaces;
  // references do).
  const mention = useMemo(() => {
    return async (query: string): Promise<MentionItem[]> => {
      const mine = ++filesAsked.current;
      if (query.startsWith("session:")) {
        const items = await sessionMentions(app, query.slice("session:".length));
        return mine === filesAsked.current ? items : [];
      }
      if (!projectRoot) return [];
      const { files } = await app.call<{ files: { path: string; size?: number }[] }>("harnix.files", {
        project_root: projectRoot,
        query,
      });
      if (mine !== filesAsked.current) return [];
      return files.map((f) => ({
        id: f.path,
        label: f.size != null ? `${(f.size / 1024).toFixed(1)} KB` : undefined,
        insert: `@${f.path} `,
      }));
    };
  }, [app, projectRoot]);

  const upload = useCallback(
    async (file: File): Promise<Attached> => {
      try {
        const path = await app.upload(file);
        const preview = file.type.startsWith("image/") && file.size < 2_000_000 ? await dataUrl(file) : undefined;
        return { label: file.name, preview, attachment: { kind: "file", path } };
      } catch (e) {
        app.notice(t("composer.uploadFailed", { error: errorText(e) }), "error");
        throw e;
      }
    },
    [app, t],
  );

  const flowBlocks = registry.at("flow.block").map((c) => ({
    id: c.id,
    match: c.match,
    render: (block: Parameters<typeof c.render>[0]) => (
      <Contributed registry={registry} point="flow.block" contribution={c}>
        {c.render(block)}
      </Contributed>
    ),
  }));
  const toolRows = registry.at("tool.row").map((c) => ({
    id: c.id,
    match: c.match,
    render: (block: Parameters<typeof c.render>[0]) => (
      <Contributed registry={registry} point="tool.row" contribution={c}>
        {c.render(block)}
      </Contributed>
    ),
  }));

  return (
    <ChatPanel
      sessionId={current.sessionId}
      instance={current.instance}
      space={s.space}
      sendExtras={referencesIn}
      onController={bind}
      header={false}
      placeholder={t("composer.placeholder")}
      hint={t("composer.hint")}
      disabled={s.connection !== "open" && s.connection !== "degraded"}
      draftKey={keyOf(current.instance, current.sessionId)}
      commands={commands}
      mention={mention}
      upload={upload}
      composer={bindComposer}
      features={{ voice: false, modelPicker: false, contextRing: false, options: false }}
      renderers={{ flowBlocks, toolRows }}
    />
  );
}

/** `@session:<instance>/<id>` in a message: the sessions it refers to. */
function referencesIn(text: string): Record<string, unknown> {
  const references = [...text.matchAll(/@session:([\w.-]+)\/([\w-]+)/g)].map((m) => ({ instance: m[1], session_id: m[2] }));
  return references.length ? { references } : {};
}

/** Sessions of every space whose name matches, for `@session:`. */
async function sessionMentions(app: App, query: string): Promise<MentionItem[]> {
  const { instances } = await app.call<{ instances: { id: string; status: { state: string } }[] }>("harnix.instances.list", {});
  const q = query.toLowerCase();
  const items: MentionItem[] = [];
  for (const instance of instances.filter((i) => i.status.state === "ready")) {
    const { sessions } = await app
      .call<{ sessions: { session_id: string; name?: string | null; space?: string }[] }>("harnix.sessions", { instance: instance.id })
      .catch(() => ({ sessions: [] }));
    for (const row of sessions) {
      const name = row.name || row.session_id;
      if (q && !name.toLowerCase().includes(q) && !row.session_id.startsWith(q)) continue;
      items.push({
        id: `session:${instance.id}/${row.session_id}`,
        label: `${name} · ${row.space ?? "sessions"}`,
        insert: `@session:${instance.id}/${row.session_id} `,
      });
    }
  }
  return items.slice(0, 20);
}

/** The `command` point: every contribution's candidates for an empty query, in order. */
function useCommands(app: App): ChatCommand[] {
  const registry = useRegistry(app.registry);
  const hello = useSyncExternalStore(app.subscribe, () => app.snapshot.hello);
  const [commands, setCommands] = useState<ChatCommand[]>([]);
  useEffect(() => {
    let alive = true;
    void Promise.all(
      registry.at("command").map((c) =>
        Promise.resolve()
          .then(() => c.complete(""))
          .catch((e: unknown) => {
            registry.fail("command", c, e);
            return [];
          }),
      ),
    ).then((lists) => {
      if (!alive) return;
      const seen = new Set<string>();
      const out: ChatCommand[] = [];
      for (const candidate of lists.flat()) {
        const name = candidate.replacement.replace(/^\//, "").trim();
        if (!name || seen.has(name)) continue;
        seen.add(name);
        out.push({ name, description: candidate.description });
      }
      setCommands(out.slice(0, 200));
    });
    return () => {
      alive = false;
    };
  }, [registry, hello, registry.getVersion()]);
  return commands;
}

function dataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

/* ── details ──────────────────────────────────────────────────────────── */

function Details({ app, s }: { app: App; s: ShellState }) {
  const t = useT();
  const registry = useRegistry(app.registry);
  const space = spacesOf(app).find((c) => c.id === s.space);
  const tabs = space?.tabs ?? [];
  const [tab, setTab] = useState<string>("details");
  // A detail revealed from the flow is what the reader wants to see now.
  useEffect(() => {
    if (s.detail) setTab("details");
  }, [s.detail]);
  useEffect(() => {
    setTab(tabs[0]?.id ?? "details");
    // Each space starts on its first tab.
  }, [s.space]);
  const item = s.detail;
  const panel = item ? registry.claim("details.panel", item) : null;
  const shown = tabs.find((x) => x.id === tab);
  const title = shown ? shown.title() : item ? (panel?.title?.(item) ?? item.name ?? t("details.title")) : t("details.title");
  return (
    <aside id="details" aria-hidden={!s.detailsOpen}>
      <div className="head">
        <span className="title truncate">{title}</span>
        <button type="button" className="icon-btn close" title={t("common.close")} onClick={() => app.toggleDetails(false)}>
          <Glyph name="close" className="glyph" />
        </button>
      </div>
      {tabs.length ? (
        <div className="tabs" role="tablist">
          {tabs.map((x) => (
            <button type="button" role="tab" key={x.id} data-tab={x.id} aria-selected={tab === x.id} className={tab === x.id ? "on" : ""} onClick={() => setTab(x.id)}>
              {x.title()}
            </button>
          ))}
          <button type="button" role="tab" data-tab="details" aria-selected={tab === "details"} className={tab === "details" ? "on" : ""} onClick={() => setTab("details")}>
            {t("details.title")}
          </button>
        </div>
      ) : null}
      <div className="body">
        {shown && space ? (
          <Contributed registry={registry} point="space" contribution={space}>
            {shown.render()}
          </Contributed>
        ) : !item ? (
          <div className="empty">{t("details.empty")}</div>
        ) : panel ? (
          <Contributed registry={registry} point="details.panel" contribution={panel}>
            {panel.render(item)}
          </Contributed>
        ) : (
          <div className="empty">{t("details.noPanel")}</div>
        )}
      </div>
    </aside>
  );
}

/* ── modal host ───────────────────────────────────────────────────────── */

function ModalHost({ app, s }: { app: App; s: ShellState }) {
  const t = useT();
  const modal = s.modal;
  return (
    <div
      id="veil"
      className={modal ? "on" : ""}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) app.closeModal();
      }}
    >
      {modal ? (
        <div id="modal" role="dialog" aria-modal="true" className={modal.wide ? "wide" : ""}>
          <div className="head">
            <span className="title">{modal.title}</span>
            <button type="button" className="icon-btn close" title={t("common.close")} onClick={() => app.closeModal()}>
              <Glyph name="close" className="glyph" />
            </button>
          </div>
          {modal.content}
        </div>
      ) : null}
    </div>
  );
}
