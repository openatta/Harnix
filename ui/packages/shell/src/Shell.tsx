/**
 * The frame: banner, three columns and the modal host. A space is the three columns together —
 * its own left column, a centre that is the conversation unless it says otherwise, and its right
 * tabs — inside the chrome that stays: the brand and the avatar. Beside the centre a space may open
 * a second side panel (`secondary`, xops-04 §7): the details of what is selected on the right. Every
 * side column is dragged wider or narrower, and remembers it.
 *
 * What is inside the conversation column is `ChatPanel`, the conversation trunk shared with
 * IPDrun.
 */

import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from "react";
import { errorText } from "@harnix/core";
import type { BlockOf, SessionController, SessionState } from "@harnix/core/state";
import {
  ChatComposer,
  ChatPanel,
  Contributed,
  listenForLayers,
  modalUp,
  t,
  useLayer,
  useRegistry,
  useT,
  ComposerNote,
  askedTool,
  rowTitle,
  toolAsks,
  type Attached,
  type ChatIds,
  type ComposerApi,
  type MentionItem,
  type ModelOption,
  type SpaceContribution,
  type SpaceSecondary,
  type ToolRowContribution,
} from "@harnix/react";

import { ask, detailsWidthOf, keyOf, later, orderTabs, safeTabs, secondaryWidthOf, type App, type OpenModal, type ShellState, type Widths } from "./app.ts";
import { useBrand } from "./brand.ts";
import { referencesIn, sessionMentions, useCommands, withContributed } from "./commands.ts";
import { followEnvelopes } from "./envelope.tsx";
import { Glyph } from "./glyph.tsx";
import { sessionMenu } from "./dialogs.tsx";
import { MenuHost, menuBelow } from "./Menu.tsx";
import { useRovingTabIndex } from "./roving.ts";
import { openSettings } from "./settings.tsx";
import { Sidebar, spacesOf } from "./Sidebar.tsx";
import { SYSTEM, openInSystem, systemOffered } from "./system/index.tsx";

export function Shell({ app }: { app: App }) {
  const s = useSyncExternalStore(app.subscribe, app.getSnapshot);
  useTheme(s.theme);
  useShortcuts(app);
  useBrand(app);
  // The right column's bounds, and a width given as a share, follow the window.
  useWindowWidth();
  const narrow = useNarrow(app, s);
  // Runs after every child has drawn: whoever waits for this state to be on the page can go on.
  useEffect(() => app.drew());
  const space = spacesOf(app).find((c) => c.id === s.space);
  const secondary = secondaryOf(app, space);
  return (
    <>
      <Banner app={app} s={s} />
      <div
        id="frame"
        data-sidebar={app.sidebarShown(s)}
        data-drawer={s.narrow && s.drawer ? "" : undefined}
        data-narrow={app.narrowDetails() === "beside" ? "beside" : undefined}
        data-details={app.detailsOpen(s) ? "open" : "closed"}
        data-details-max={app.detailsOpen(s) && s.detailsMax ? "" : undefined}
        data-secondary={secondary ? "open" : undefined}
        data-secondary-max={secondary && app.secondaryMax(s) ? "" : undefined}
        data-space={s.space}
        style={
          {
            "--sidebar-width": `${s.widths.side}px`,
            "--details-width": `${detailsWidthOf(s, space)}px`,
            "--secondary-width": `${secondaryWidthOf(s, secondary?.width)}px`,
          } as CSSProperties
        }
        onKeyDown={narrow.onKeyDown}
      >
        <Sidebar app={app} s={s} />
        <Resizer app={app} column="side" />
        <div id="centre">
          <Main app={app} s={s} />
          {secondary && space ? <Secondary app={app} space={space} panel={secondary} max={app.secondaryMax(s)} /> : null}
        </div>
        {space || s.detail ? <Details app={app} s={s} /> : <aside id="details" aria-hidden inert />}
        {narrow.veil ? <div className="drawer-veil" data-testid="drawer-veil" aria-hidden="true" onClick={narrow.closeTop} /> : null}
      </div>
      <ModalHost app={app} s={s} />
      {/* Where a layer drawn outside its own place goes: an anchored menu, a dialog's own dialog. */}
      <div id="layers" />
      <MenuHost />
    </>
  );
}

/**
 * The edge between a side column and the centre: drag it, and the column follows within its
 * bounds; double-click, and it goes back to its own width.
 */
function Resizer({ app, column }: { app: App; column: keyof Widths }) {
  const t = useT();
  const start = (event: ReactPointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    const from = event.clientX;
    const front = spacesOf(app).find((c) => c.id === app.snapshot.space);
    const width =
      column === "details"
        ? detailsWidthOf(app.snapshot, front)
        : column === "secondary"
          ? secondaryWidthOf(app.snapshot, secondaryOf(app, front)?.width)
          : app.snapshot.widths[column];
    const move = (e: PointerEvent) => app.setWidth(column, column === "side" ? width + (e.clientX - from) : width - (e.clientX - from));
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.documentElement.classList.remove("resizing");
    };
    document.documentElement.classList.add("resizing");
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };
  return (
    <div
      className={`resizer ${column}`}
      role="separator"
      aria-orientation="vertical"
      title={t("app.resize")}
      data-testid={`resize-${column}`}
      onPointerDown={start}
      onDoubleClick={() => app.setWidth(column, null)}
    />
  );
}

/* ── narrow windows ───────────────────────────────────────────────────── */

/** Whether a media query matches, as the window changes. */
function useMedia(query: string): boolean {
  const matches = () => typeof matchMedia === "function" && matchMedia(query).matches;
  const [on, setOn] = useState(matches);
  useEffect(() => {
    if (typeof matchMedia !== "function") return;
    const list = matchMedia(query);
    const change = () => setOn(list.matches);
    change();
    list.addEventListener("change", change);
    return () => list.removeEventListener("change", change);
  }, [query]);
  return on;
}

/** A ref to an element the frame draws somewhere below it, found when it is asked for. */
function byId(id: string): RefObject<HTMLElement | null> {
  return {
    get current() {
      return document.getElementById(id);
    },
  } as RefObject<HTMLElement | null>;
}

const SIDE = byId("side");
const DETAILS = byId("details");

/**
 * At 960px and below (Helm §1) the sidebar is the rail, and the toggle, the rail's entry or ⌘\
 * open it as a drawer over the page; an open right column is a drawer from the right (unless the
 * profile keeps it beside, `narrow_details`, and there is room). Each drawer is a layer
 * (`@harnix/react`'s `layers.ts`) that is not modal: Escape closes the top one and the page stays live. A veil under
 * them closes them on a click, and the sidebar's drawer closes once a session or a space is picked
 * in it.
 */
function useNarrow(app: App, s: ShellState): { veil: boolean; closeTop: () => void; onKeyDown: (event: ReactKeyboardEvent) => void } {
  const narrow = useMedia("(max-width: 960px)");
  const roomy = useMedia("(min-width: 720px)");
  useLayoutEffect(() => app.setNarrow(narrow), [app, narrow]);
  const drawer = s.narrow && s.drawer;
  const over = s.narrow && app.detailsOpen(s) && !s.detailsMax && !(app.narrowDetails() === "beside" && roomy);
  const side = useLayer(SIDE, { open: drawer, modal: false, close: () => app.closeDrawer() });
  const details = useLayer(DETAILS, { open: over, modal: false, close: () => app.toggleDetails(false) });
  // Picked in the drawer: what was picked is what the person wants to see now.
  const picked = `${s.space}\n${s.current ? keyOf(s.current.instance, s.current.sessionId) : ""}`;
  const last = useRef(picked);
  useEffect(() => {
    if (last.current === picked) return;
    last.current = picked;
    app.closeDrawer();
  }, [app, picked]);
  return {
    veil: drawer || over,
    closeTop: () => (drawer ? app.closeDrawer() : app.toggleDetails(false)),
    // Escape inside a drawer that nothing in it took (a layer's own handler; see `useLayer`).
    onKeyDown: (event) => {
      const target = event.target as Node;
      if (drawer && SIDE.current?.contains(target)) side.onKeyDown(event);
      else if (over && DETAILS.current?.contains(target)) details.onKeyDown(event);
    },
  };
}

/* ── theme and keys ───────────────────────────────────────────────────── */

/** Draw again when the window's width changes, at most once a frame. */
function useWindowWidth(): void {
  const [, bump] = useState(0);
  useEffect(() => {
    let frame = 0;
    const changed = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => bump((n) => n + 1));
    };
    window.addEventListener("resize", changed);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("resize", changed);
    };
  }, []);
}

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

/**
 * The shell's keys. Escape belongs to the layers (`@harnix/react`'s `layers.ts`): their listener comes first and
 * takes it while one is open. The shortcuts do nothing while a dialog is: what is under it is not
 * the page's to act on.
 */
function useShortcuts(app: App) {
  useEffect(() => listenForLayers(), []);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (modalUp()) return;
      const mod = event.metaKey || event.ctrlKey;
      if (mod && event.key.toLowerCase() === "k") {
        // `[shell] new_session = false` (ipdrun-46): ⌘K is not the shell's — left to the page.
        if (!app.startsSessions()) return;
        event.preventDefault();
        document.querySelector<HTMLButtonElement>("#new")?.click();
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

/**
 * The notice in front, and how many wait behind it. A live region: a reader hears an error at
 * once (`alert`), anything else when it is free to (`status`).
 */
function Banner({ app, s }: { app: App; s: ShellState }) {
  const t = useT();
  const error = s.banner?.tone === "error";
  const waiting = s.notices.length - 1;
  return (
    <div
      id="banner"
      className={s.banner ? "on" : ""}
      data-state={s.banner ? "open" : "closed"}
      data-tone={s.banner?.tone}
      role={error ? "alert" : "status"}
      aria-live={error ? "assertive" : "polite"}
      aria-atomic="true"
    >
      {s.banner ? (
        <>
          <Glyph name="alert" className="glyph" />
          <span className="text">{s.banner.text}</span>
          {waiting > 0 ? (
            <span className="waiting" data-testid="banner-waiting">
              {t("layers.noticesWaiting", { count: waiting })}
            </span>
          ) : null}
          <button type="button" className="icon-btn close" title={t("banner.dismiss")} aria-label={t("banner.dismiss")} onClick={() => app.dismissBanner()}>
            <Glyph name="close" className="glyph" />
          </button>
        </>
      ) : null}
    </div>
  );
}

/* ── the conversation column ──────────────────────────────────────────── */

function Main({ app, s }: { app: App; s: ShellState }) {
  const t = useT();
  const [controller, setController] = useState<SessionController | null>(null);
  const session = useControllerState(controller);
  const space = spacesOf(app).find((c) => c.id === s.space);
  const center = space?.center ?? "chat";
  // An overview space shows its conversation on demand: after something is asked, or when the
  // reader goes back to it.
  const [talking, setTalking] = useState(false);
  useEffect(() => setTalking(false), [s.space]);
  if (!space) {
    return (
      <section id="main" className="overview" data-space="">
        <div id="overview" data-testid="overview">
          <div className="empty no-spaces" data-testid="no-spaces">
            {t("shell.noSpaces")}
          </div>
        </div>
      </section>
    );
  }
  if (center !== "chat" && !(talking && s.current)) {
    return (
      <section id="main" className="overview" data-space={s.space}>
        <Header app={app} s={s} session={null} onBack={null} onTalk={s.current && center !== "hidden" ? () => setTalking(true) : null} />
        <div id="overview" data-testid="overview">
          <Overview app={app} space={space} onAsked={() => setTalking(true)} />
        </div>
      </section>
    );
  }
  const empty = !s.current;
  return (
    <section id="main" className={empty ? "empty" : ""} data-space={s.space} data-state={empty ? "empty" : "open"}>
      <Header app={app} s={s} session={session} onBack={center !== "chat" ? () => setTalking(false) : null} onTalk={null} />
      {s.current ? (
        <Conversation key={keyOf(s.current.instance, s.current.sessionId)} app={app} s={s} onController={setController} />
      ) : (
        <Hero app={app} />
      )}
    </section>
  );
}

/** An overview space's centre: what it registered, with an ask box over it if it has a conversation. */
function Overview({ app, space, onAsked }: { app: App; space: SpaceContribution & { owner: string }; onAsked: () => void }) {
  const registry = useRegistry(app.registry);
  return (
    <Contributed registry={registry} point="space" contribution={space}>
      <div className={`dash-in${space.center === "hidden" ? " full" : ""}`}>
        {space.overview?.() ?? null}
        {space.center !== "hidden" && space.newSession !== false ? <FirstMessage app={app} variant="ask" onSent={onAsked} placeholder={space.placeholder?.()} /> : null}
      </div>
    </Contributed>
  );
}

/** "Good morning" by the clock. */
export function greeting(t: (k: string) => string): string {
  const hour = new Date().getHours();
  return hour < 12 ? t("greet.morning") : hour < 18 ? t("greet.afternoon") : t("greet.evening");
}

/**
 * The first message of a session in this space: it creates the session, then sends. The composer is
 * the conversation's own, with no session behind it yet: the model picked here is the new
 * session's, set before the first turn.
 */
function FirstMessage({ app, onSent, variant = "composer", placeholder }: { app: App; onSent?: () => void; variant?: "composer" | "ask"; placeholder?: string }) {
  const t = useT();
  const commands = useCommands(app);
  const models = useModels(app);
  const engineModel = useSyncExternalStore(app.subscribe, () => app.defaultModel());
  const ready = useSyncExternalStore(app.subscribe, () => app.modelReady());
  const switchable = useSyncExternalStore(app.subscribe, () => setModelMethod(app) !== null);
  const connection = useSyncExternalStore(app.subscribe, () => app.snapshot.connection);
  const space = useSyncExternalStore(app.subscribe, () => app.snapshot.space);
  const [picked, setPicked] = useState<ModelOption | null>(null);
  const upload = useUpload(app);
  const bind = useBindComposer(app);
  const mention = useMemo(
    () => async (query: string) => withContributed(app, query, query.startsWith("session:") ? await sessionMentions(app, query.slice("session:".length)) : []),
    [app],
  );
  const send = async (message: string, attachments: unknown[]): Promise<boolean> => {
    if (!app.modelReady()) {
      openSettings(app, "models");
      return false;
    }
    try {
      const key = await app.create({});
      const target = { instance: key.instance, session_id: key.sessionId };
      if (picked && picked.id !== engineModel) {
        await setModel(app, target, picked);
        chosenModels.set(keyOf(key.instance, key.sessionId), picked.id);
      }
      await app.sessionCall("harnix.send", target, { message, attachments, ...referencesIn(message) });
      onSent?.();
      return true;
    } catch (e) {
      app.notice(errorText(e), "error");
      return false;
    }
  };
  return (
    <div className={`hx-chat ${variant === "ask" ? "ask" : "first-message"}`} data-testid={variant === "ask" ? "ask" : "first-message"}>
      <ChatComposer
        state={null}
        send={send}
        upload={upload}
        interrupt={() => undefined}
        sendMode="stop"
        disabled={connection !== "open" && connection !== "degraded"}
        placeholder={placeholder ?? t("composer.placeholder")}
        quickReplies={NO_REPLIES}
        notice={ready ? undefined : <ModelSetupNote app={app} />}
        hint={variant === "ask" ? undefined : t("composer.hint")}
        commands={commands}
        mention={mention}
        contextRing={null}
        options={null}
        attachments
        voice={false}
        model={picked?.id ?? engineModel}
        modelUnset={!ready}
        models={models}
        onModel={switchable ? setPicked : undefined}
        draftKey={`new:${space}:${variant}`}
        bind={bind}
      />
    </div>
  );
}

const NO_REPLIES: never[] = [];

/** The host's composer: whichever input is on the page now. One leaving clears it only if it is
 *  still the one bound — the next one may have bound already. An open session's (`session`, its key)
 *  is also bound as that session's: text kept for it goes in. */
function useBindComposer(app: App, session?: string): (api: ComposerApi | null) => void {
  const mine = useRef<ComposerApi | null>(null);
  return useCallback(
    (api: ComposerApi | null) => {
      if (api) {
        mine.current = api;
        app.composer = api;
        if (session !== undefined) app.composerBound(session, api);
      } else {
        if (app.composer === mine.current) app.composer = null;
        if (session !== undefined && mine.current) app.composerGone(session, mine.current);
        mine.current = null;
      }
    },
    [app, session],
  );
}

/**
 * The method a session's model is changed by: `harnix.sessions.setModel` when the profile opens it
 * (`[shell] model_switch`), for a session of the front space; else the system space's, so only when
 * that space is kept. Null when neither is reachable: the menu shows the model and cannot change it.
 */
function setModelMethod(app: App): string | null {
  const reachable = app.snapshot.reachable;
  if (reachable.includes("harnix.sessions.setModel")) return "harnix.sessions.setModel";
  if (reachable.includes("harnix.system.model")) return "harnix.system.model";
  return null;
}

async function setModel(app: App, target: { instance: string; session_id: string }, option: ModelOption): Promise<void> {
  const method = setModelMethod(app);
  if (!method) return;
  const params = { ...target, ...modelParams(option) };
  await app.call(method, method === "harnix.sessions.setModel" ? { ...params, space: app.claim() } : params);
}

/** The model each session was switched to here, by session key: the session's own record does not
 *  say which model it runs on. */
const chosenModels = new Map<string, string>();

function modelParams(option: ModelOption): Record<string, string> {
  return option.provider ? { model: option.id, provider: option.provider } : { model: option.id };
}

/** What the model menu offers: the default model first, then each provider's default and allowed models. */
function useModels(app: App): ModelOption[] {
  const providers = useSyncExternalStore(app.subscribe, () => app.snapshot.providers);
  const hello = useSyncExternalStore(app.subscribe, () => app.snapshot.hello);
  return useMemo(() => {
    const out: ModelOption[] = [];
    const seen = new Set<string>();
    const add = (id: string | null | undefined, provider?: string) => {
      if (!id || seen.has(id)) return;
      seen.add(id);
      out.push(provider ? { id, provider } : { id });
    };
    const entries = Object.entries(providers?.providers ?? {});
    const first = providers?.default_provider;
    entries.sort(([a], [b]) => (a === first ? -1 : b === first ? 1 : 0));
    for (const [provider, config] of entries) {
      add(config.default_model, provider);
      for (const id of config.models ?? []) add(id, provider);
    }
    if (!entries.length) add(typeof hello?.engine?.model === "string" ? hello.engine.model : null);
    return out;
  }, [providers, hello]);
}

/** Over the composer while no model is configured: what is missing, and the way to it. */
function ModelSetupNote({ app }: { app: App }) {
  const t = useT();
  return (
    <div className="hx-cmp-note hx-cmp-note-warn model-setup" data-testid="model-setup">
      <Glyph name="key" className="glyph" />
      <span className="text">{t("model.setupNeeded")}</span>
      <button type="button" className="btn sm primary" data-testid="model-setup-open" onClick={() => openSettings(app, "models")}>
        {t("model.setupAction")}
      </button>
    </div>
  );
}

/** Upload a file for the next message; a failure is said, then rethrown so the composer drops it. */
function useUpload(app: App): (file: File) => Promise<Attached> {
  const t = useT();
  return useCallback(
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
}

const NOTHING = () => () => {};

function useControllerState(controller: SessionController | null): SessionState | null {
  const subscribe = controller ? controller.subscribe : NOTHING;
  return useSyncExternalStore(subscribe, () => (controller ? controller.getSnapshot() : null));
}

/** A space with no session open: the greeting, the composer that starts one, suggestions. */
function Hero({ app }: { app: App }) {
  const t = useT();
  const space = spacesOf(app).find((c) => c.id === app.snapshot.space);
  const suggestions = space?.suggestions?.() ?? [];
  const [seed, setSeed] = useState(0);
  return (
    <div id="scroll" className="hero-wrap">
      <div id="hero" className="hero">
        <div className="greet">
          <Glyph name="brand" className="logo" />
          <span>{t("hero.greeting", { greeting: greeting(t) })}</span>
        </div>
        <div className="hero-sub">{space?.subtitle?.() ?? space?.title() ?? t("hero.body")}</div>
      </div>
      {space?.newSession === false ? (
        <div className="hero-sub" data-testid="no-new-session">
          {space.placeholder?.()}
        </div>
      ) : (
        <FirstMessage key={seed} app={app} placeholder={space?.placeholder?.()} />
      )}
      {suggestions.length && space?.newSession !== false ? (
        <div className="chips">
          {suggestions.map((text) => (
            <button
              type="button"
              key={text}
              className="chip"
              onClick={async () => {
                try {
                  const key = await app.create({});
                  await app.sessionCall("harnix.send", { instance: key.instance, session_id: key.sessionId }, { message: text });
                  setSeed((n) => n + 1);
                } catch (e) {
                  app.notice(errorText(e), "error");
                }
              }}
            >
              {text}
            </button>
          ))}
        </div>
      ) : null}
      {space?.notice ? <div className="disc">{space.notice()}</div> : null}
      <HomeCards app={app} />
    </div>
  );
}

/** The `home.card` registrations, under a new conversation's composer: what the product wants in
 *  front of someone about to start something. */
function HomeCards({ app }: { app: App }) {
  const registry = useRegistry(app.registry);
  const cards = [...registry.at("home.card")].sort((a, b) => (a.order ?? 100) - (b.order ?? 100));
  if (!cards.length) return null;
  return (
    <div className="dgrid home-cards" data-testid="home-cards">
      {cards.map((card) => (
        <section key={card.id} className="dcard home-card" data-card={card.id}>
          {card.title ? <div className="dt">{card.title()}</div> : null}
          <Contributed registry={registry} point="home.card" contribution={card}>
            {card.render()}
          </Contributed>
        </section>
      ))}
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

function Header({
  app,
  s,
  session,
  onBack,
  onTalk,
}: {
  app: App;
  s: ShellState;
  session: SessionState | null;
  onBack: (() => void) | null;
  onTalk: (() => void) | null;
}) {
  const t = useT();
  const space = spacesOf(app).find((c) => c.id === s.space);
  const row = s.current ? s.sessions.find((r) => r.session_id === s.current!.sessionId && r.instance === s.current!.instance) : null;
  const name = row?.name ?? session?.session?.name ?? null;
  const scene = session?.session?.scene ?? row?.scene;
  const overview = space?.center && space.center !== "chat" && !onBack;
  const registry = useRegistry(app.registry);
  const actions = space?.headerActions
    ? ask(registry, space, () => space.headerActions!(s.current ? { instance: s.current.instance, sessionId: s.current.sessionId } : null), null)
    : null;
  return (
    <header id="head" className="topbar" data-tauri-drag-region="deep">
      {onBack ? (
        <button type="button" className="ib back" data-testid="back-to-overview" title={space?.title()} onClick={onBack}>
          <Glyph name="chevronRight" className="glyph flip" />
        </button>
      ) : null}
      {row && !overview ? (
        <button
          type="button"
          className="title-btn"
          data-testid="head-title"
          title={t("sidebar.sessionActions")}
          aria-haspopup="menu"
          onClick={(e) => sessionMenu(app, row, { origin: e.currentTarget, at: menuBelow(e.currentTarget) })}
        >
          <span className="title truncate">{name || t("sidebar.untitled")}</span>
          <Glyph name="chevronDown" className="glyph" />
        </button>
      ) : (
        <div className="title truncate">{overview ? space?.title() : s.current ? name || t("sidebar.untitled") : space?.newSession === false ? space.title() : t("sidebar.newSession")}</div>
      )}
      <div className="meta">
        {space ? (
          <span className="pill acc" data-testid="space-pill">
            {space.title()}
            {space.subtitle ? <span className="sub"> · {space.subtitle()}</span> : null}
          </span>
        ) : null}
        {!overview && scene ? <span className="pill">{scene}</span> : null}
        {!overview ? usagePills(t, session?.usage ?? null) : null}
      </div>
      <div className="spacer" />
      {onTalk ? (
        <button type="button" className="tpill" data-testid="to-conversation" onClick={onTalk}>
          <Glyph name="chat" className="glyph" />
          {t("home.continue")}
        </button>
      ) : null}
      {actions != null && actions !== false && space ? (
        <Contributed registry={registry} point="space" contribution={space}>
          <div className="head-actions" data-testid="head-actions" style={{ display: "flex", alignItems: "center", gap: 4 }}>
            {actions}
          </div>
        </Contributed>
      ) : null}
      {row && !overview && s.space !== SYSTEM && systemOffered() ? (
        <button
          type="button"
          className="ib"
          data-testid="head-in-system"
          title={t("head.inSystem")}
          aria-label={t("head.inSystem")}
          onClick={() => void openInSystem(app, { instance: row.instance ?? "system", sessionId: row.session_id, space: s.space })}
        >
          <Glyph name="cpu" className="glyph" />
        </button>
      ) : null}
      <button type="button" className="ib" data-testid="theme-toggle" title={t("app.theme")} onClick={() => app.setTheme(isDark() ? "light" : "dark")}>
        <Glyph name={isDark() ? "sun" : "moon"} className="glyph" />
      </button>
      <button type="button" className="ib" data-testid="details-toggle" title={t("app.details")} onClick={() => app.toggleDetails()}>
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
  const bindComposer = useBindComposer(app, keyOf(current.instance, current.sessionId));
  useEffect(() => (controller ? followEnvelopes(app, controller) : undefined), [app, controller]);

  const sessionKey = keyOf(current.instance, current.sessionId);
  const sessionState = useControllerState(controller);
  const models = useModels(app);
  const switchable = setModelMethod(app) !== null;
  const [chosen, setChosen] = useState(() => chosenModels.get(sessionKey) ?? null);
  const recorded = sessionState?.session?.model;
  const model = chosen ?? (typeof recorded === "string" ? recorded : null) ?? app.defaultModel() ?? undefined;
  const onModel = useCallback(
    async (option: ModelOption) => {
      try {
        await setModel(app, { instance: current.instance, session_id: current.sessionId }, option);
        chosenModels.set(sessionKey, option.id);
        setChosen(option.id);
      } catch (e) {
        app.notice(errorText(e), "error");
        throw e;
      }
    },
    [app, current.instance, current.sessionId, sessionKey],
  );

  const commands = useCommands(app);
  const projectRoot = s.sessions.find((r) => r.session_id === current.sessionId)?.project_root ?? null;
  const filesAsked = useRef(0);
  // `@` offers the project's files and — after `@session:` — sessions of any space, which go to
  // the turn as a copy of what was said in them (concept §5.4: sessions do not cross spaces;
  // references do).
  const mention = useMemo(() => {
    return async (query: string): Promise<MentionItem[]> => {
      const mine = ++filesAsked.current;
      if (query.startsWith("session:")) {
        const items = await withContributed(app, query, await sessionMentions(app, query.slice("session:".length)));
        return mine === filesAsked.current ? items : [];
      }
      const files = projectRoot
        ? (
            await app.call<{ files: { path: string; size?: number }[] }>("harnix.files", {
              project_root: projectRoot,
              query,
            })
          ).files
        : [];
      if (mine !== filesAsked.current) return [];
      const items = await withContributed(
        app,
        query,
        files.map((f) => ({
          id: f.path,
          label: f.size != null ? `${(f.size / 1024).toFixed(1)} KB` : undefined,
          insert: `@${f.path} `,
        })),
      );
      return mine === filesAsked.current ? items : [];
    };
  }, [app, projectRoot]);

  const upload = useUpload(app);

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
    title: c.title,
    render: (block: Parameters<typeof c.render>[0]) => (
      <Contributed registry={registry} point="tool.row" contribution={c}>
        {c.render(block)}
      </Contributed>
    ),
  }));

  // What the front space says about this session: read at every draw, each one failing on its own.
  const space = spacesOf(app).find((c) => c.id === s.space);
  const ref = { instance: current.instance, sessionId: current.sessionId };
  const readOnly = space?.readOnly ? ask(registry, space, () => space.readOnly!(ref), undefined) : undefined;
  const pinned = space?.pinned ? ask(registry, space, () => space.pinned!(ref), null) : null;
  // Nothing said yet in it: the space may say what this conversation is for.
  const blank = !!sessionState && !sessionState.loading && !sessionState.failed && sessionState.blocks.length === 0 && !sessionState.turn && !sessionState.queue.length;
  const emptyHint = blank && space?.emptyConversation ? ask(registry, space, () => space.emptyConversation!(ref), null) : null;
  const spaceNotice = space?.composerNotice ? ask(registry, space, () => space.composerNotice!(ref), null) : null;
  const ids = space?.ids ? globalIds(app, space, ask(registry, space, () => space.ids!(), undefined)) : undefined;
  const steerable = space?.composer?.steer === true;
  const running = sessionState?.turn != null || sessionState?.session?.turn_state === "running";
  const [steer, setSteer] = useState(() => steering.has(sessionKey));
  const toggleSteer = () => {
    if (steer) steering.delete(sessionKey);
    else steering.add(sessionKey);
    setSteer(!steer);
  };
  const ready = app.modelReady();
  const notice =
    !ready || (spaceNotice != null && spaceNotice !== false) ? (
      <>
        {ready ? null : <ModelSetupNote app={app} />}
        {spaceNotice != null && spaceNotice !== false && space ? (
          <ComposerNote icon="shield" testId="space-notice">
            <Contributed registry={registry} point="space" contribution={space}>
              {spaceNotice}
            </Contributed>
          </ComposerNote>
        ) : null}
      </>
    ) : undefined;

  return (
    <>
      <ChatPanel
        sessionId={current.sessionId}
        instance={current.instance}
        space={app.claim(s.space)}
        sendExtras={referencesIn}
        onController={bind}
        header={false}
        placeholder={spacesOf(app).find((c) => c.id === s.space)?.placeholder?.() ?? t("composer.placeholder")}
        hint={t("composer.hint")}
        disabled={s.connection !== "open" && s.connection !== "degraded"}
        draftKey={keyOf(current.instance, current.sessionId)}
        commands={commands}
        mention={mention}
        upload={upload}
        composer={bindComposer}
        notice={notice}
        readOnly={readOnly || undefined}
        pinned={
          (pinned != null && pinned !== false) || (emptyHint != null && emptyHint !== false) ? (
            space ? (
              <Contributed registry={registry} point="space" contribution={space}>
                {pinned != null && pinned !== false ? pinned : null}
                {emptyHint != null && emptyHint !== false ? (
                  <div className="empty-conversation" data-testid="empty-conversation">
                    {emptyHint}
                  </div>
                ) : null}
              </Contributed>
            ) : undefined
          ) : undefined
        }
        ids={ids}
        onBusy={steerable && steer ? "steer" : undefined}
        composerTools={
          steerable ? (
            <button
              type="button"
              role="switch"
              className={`hx-cmp-model steer${steer ? " on" : ""}`}
              data-testid="conv-steer"
              aria-checked={steer}
              disabled={!running}
              title={t("composer.steerHint")}
              onClick={toggleSteer}
            >
              {t("composer.steer")}
            </button>
          ) : undefined
        }
        model={model}
        modelUnset={!ready}
        models={models}
        onModel={switchable ? onModel : undefined}
        features={{ voice: false, contextRing: true, options: false }}
        renderers={{ flowBlocks, toolRows }}
      />
      <AskAnnouncer state={sessionState} toolRows={toolRows} />
    </>
  );
}

/**
 * An ask arriving is said aloud on its own, once: the card is in the flow's log, which a reader is
 * not told about at once, and a turn waits on it. It names the tool as the card does: by its call's
 * row (`tool.row`'s `title`), not by its wire name. The name is read at each draw, as the card's is:
 * an application's rows can register after its first ask has arrived.
 */
function AskAnnouncer({ state, toolRows }: { state: SessionState | null; toolRows: readonly ToolRowContribution[] }) {
  const t = useT();
  const heard = useRef(new Set<string>());
  const [said, setSaid] = useState<BlockOf<"ask">["prompt"] | null>(null);
  const blocks = state?.blocks ?? [];
  const asks = blocks.filter((b) => b.kind === "ask" && !b.answered);
  const fresh = asks.find((b) => b.kind === "ask" && !heard.current.has(b.prompt.prompt_id));
  useEffect(() => {
    if (!fresh || fresh.kind !== "ask") return;
    heard.current.add(fresh.prompt.prompt_id);
    setSaid(fresh.prompt);
  }, [fresh]);
  const tool = said && asks.length ? askedTool(said, toolAsks(blocks, (block) => rowTitle(block, toolRows))) : "";
  return (
    <div className="sr-only" role="alert" data-testid="ask-announce">
      {said && asks.length ? t("layers.askAnnounce", { tool: tool || t("layers.askAnnounceTool") }) : ""}
    </div>
  );
}

/** Sessions with "deliver now" on, by session key: kept in the page while it is open. */
const steering = new Set<string>();

/** A space's ids, with the `g` flag `ChatIds` needs: added when missing, and noted. */
function globalIds(app: App, space: { id: string; owner: string }, ids: ChatIds | undefined): ChatIds | undefined {
  if (!ids || !(ids.pattern instanceof RegExp)) return undefined;
  if (ids.pattern.global) return ids;
  later(() => app.registry.note({ point: "space", owner: space.owner, id: space.id, reason: t("host.idsNotGlobal") }));
  return { ...ids, pattern: new RegExp(ids.pattern.source, `${ids.pattern.flags}g`) };
}

function dataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

/* ── the second side panel ─────────────────────────────────────────── */

/** The front space's second side panel, if it has one open now; a throw counts as none. */
function secondaryOf(app: App, space: (SpaceContribution & { owner: string }) | undefined): SpaceSecondary | null {
  if (!space?.secondary) return null;
  const panel = ask(app.registry, space, () => space.secondary!(), null);
  return panel && typeof panel.render === "function" ? panel : null;
}

/**
 * What the front space has open beside the conversation: its heading, a close button, its body.
 * Closing hands it back to the space (`onClose`), which lets go of the selection; the centre stays.
 */
function Secondary({ app, space, panel, max }: { app: App; space: SpaceContribution & { owner: string }; panel: SpaceSecondary; max: boolean }) {
  const t = useT();
  const registry = useRegistry(app.registry);
  const title = ask(registry, space, () => panel.title(), "");
  return (
    <aside id="secondary" data-testid="secondary" data-key={panel.key} aria-label={title}>
      <Resizer app={app} column="secondary" />
      <div className="strip" data-tauri-drag-region="deep">
        <div className="title" data-testid="secondary-title">
          {title}
        </div>
        <div className="win">
          <button
            type="button"
            className="icon-btn"
            data-testid="secondary-max"
            title={max ? t("secondary.restore") : t("secondary.maximize")}
            aria-label={max ? t("secondary.restore") : t("secondary.maximize")}
            aria-pressed={max}
            onClick={() => app.toggleSecondaryMax()}
          >
            <Glyph name={max ? "restore" : "maximize"} className="glyph" />
          </button>
          <button
            type="button"
            className="icon-btn close"
            data-testid="secondary-close"
            title={t("common.close")}
            onClick={() => {
              ask(registry, space, () => panel.onClose?.(), undefined);
              app.redraw();
            }}
          >
            <Glyph name="close" className="glyph" />
          </button>
        </div>
      </div>
      <div className="body">
        <Contributed registry={registry} point="space" contribution={space}>
          {ask(registry, space, () => panel.render(), null)}
        </Contributed>
      </div>
    </aside>
  );
}

/* ── details ──────────────────────────────────────────────────────────── */

function Details({ app, s }: { app: App; s: ShellState }) {
  const t = useT();
  const registry = useRegistry(app.registry);
  const space = spacesOf(app).find((c) => c.id === s.space);
  const tabs = orderTabs(safeTabs(registry, space));
  const firstEnd = tabs.find((x) => x.align === "end")?.id;
  const setTab = (id: string) => app.selectTab(id);
  const item = s.detail;
  const panel = item ? registry.claim("details.panel", item) : null;
  // The selected tab is the app's, per space (the host API can choose it). A space's tabs can change
  // with what is selected: a tab that went falls back to the first.
  const current = app.shownTab(s);
  const shown = tabs.find((x) => x.id === current);
  // One Tab stop for the strip; the arrows go along it.
  const strip = useRef<HTMLDivElement>(null);
  const roving = useRovingTabIndex(strip, { items: '[role="tab"]', orientation: "horizontal", activate: true });
  return (
    // Closed it is zero wide, not gone: inert, or Tab would still walk through what is in it.
    <aside id="details" aria-hidden={!app.detailsOpen(s)} inert={!app.detailsOpen(s)}>
      <Resizer app={app} column="details" />
      <div className="strip" data-tauri-drag-region="deep">
        <div className="tabs" role="tablist" ref={strip} onKeyDown={roving.onKeyDown} onFocus={roving.onFocus}>
          {tabs.map((x) => (
            <button
              type="button"
              role="tab"
              key={x.id}
              id={`details-tab-${x.id}`}
              aria-controls="details-panel"
              data-tab={x.id}
              data-align={x.id === firstEnd ? "end" : undefined}
              aria-selected={current === x.id}
              className={current === x.id ? "on" : ""}
              onClick={() => setTab(x.id)}
            >
              {x.title()}
            </button>
          ))}
          {item || !tabs.length ? (
            <button type="button" role="tab" id="details-tab-details" aria-controls="details-panel" data-tab="details" aria-selected={current === "details"} className={current === "details" ? "on" : ""} onClick={() => setTab("details")}>
              {item && !tabs.length ? (panel?.title?.(item) ?? item.name ?? t("details.title")) : t("details.title")}
            </button>
          ) : null}
        </div>
        <div className="win">
          <button
            type="button"
            className="icon-btn"
            data-testid="details-max"
            title={s.detailsMax ? t("details.restore") : t("details.maximize")}
            aria-pressed={s.detailsMax}
            onClick={() => app.toggleDetailsMax()}
          >
            <Glyph name={s.detailsMax ? "restore" : "maximize"} className="glyph" />
          </button>
          <button type="button" className="icon-btn close" data-testid="details-close" title={t("common.close")} onClick={() => app.toggleDetails(false)}>
            <Glyph name="close" className="glyph" />
          </button>
        </div>
      </div>
      <div className="body" role="tabpanel" id="details-panel" aria-labelledby={`details-tab-${shown ? shown.id : "details"}`}>
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

/**
 * The shell's dialogs, a stack: the first over the page in `#veil`, each later one over the last in
 * a veil of its own. Each is a layer (`@harnix/react`'s `layers.ts`): the focus is in the top one, what is under it is
 * inert, and Escape or a click on its veil closes that one alone.
 */
function ModalHost({ app, s }: { app: App; s: ShellState }) {
  const [first, ...over] = s.modals;
  return (
    <>
      <div
        id="veil"
        className={first ? "on" : ""}
        data-state={first ? "open" : "closed"}
        onMouseDown={(event) => {
          if (event.target === event.currentTarget && first) app.closeModal(first);
        }}
      >
        {first ? <ModalLayer key={first.id} app={app} modal={first} first /> : null}
      </div>
      {over.map((modal, i) => (
        <div
          key={modal.id}
          className="hx-veil on"
          style={{ zIndex: 41 + i }}
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) app.closeModal(modal);
          }}
        >
          <ModalLayer app={app} modal={modal} />
        </div>
      ))}
    </>
  );
}

function ModalLayer({ app, modal, first = false }: { app: App; modal: OpenModal; first?: boolean }) {
  const t = useT();
  const box = useRef<HTMLDivElement>(null);
  const title = useId();
  // Into the dialog's own fields and buttons first; its × only when it has nothing else.
  const layer = useLayer(box, { close: () => app.closeModal(modal) });
  return (
    <div
      ref={box}
      id={first ? "modal" : undefined}
      role="dialog"
      aria-modal="true"
      aria-labelledby={title}
      tabIndex={-1}
      className={`hx-modal${modal.wide ? " wide" : ""}`}
      data-testid="modal"
      onKeyDown={layer.onKeyDown}
    >
      <div className="head">
        <span className="title" id={title}>
          {modal.title}
        </span>
        <button type="button" className="icon-btn close" title={t("common.close")} aria-label={t("common.close")} data-initial-skip onClick={() => app.closeModal(modal)}>
          <Glyph name="close" className="glyph" />
        </button>
      </div>
      {modal.content}
    </div>
  );
}
