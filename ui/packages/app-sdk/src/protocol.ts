/**
 * `harnix-ui-bridge@1`: what an application's isolated frame and the shell say to each other.
 *
 * The shell makes the frame (`sandbox="allow-scripts allow-forms"`, an opaque origin, no network)
 * and, when its page has loaded, hands it one end of a `MessageChannel` — the only window message
 * of the protocol (`Connect`), sent once, to that frame alone. Everything after goes over the
 * port: requests (answered by a response with the same `id`), responses, and events (answered by
 * nothing). The frame speaks first: `hello`, answered with what it is (`Welcome`).
 *
 * The frame never gets the page's token, its connection, its DOM, or another application's
 * names: it asks, and the shell does — what the application's manifest lets it ask for (its own
 * `app.<id>.*` and its `[interface] hub`), and nothing else. Both sides import these types, so
 * this file is the protocol's definition.
 */

export const PROTOCOL = "harnix-ui-bridge@1";

/** The shell to a frame it made, once, with the port in `ports[0]`. */
export interface Connect {
  protocol: typeof PROTOCOL;
  type: "connect";
}

export interface Request {
  kind: "request";
  id: number;
  method: string;
  params?: unknown;
}

export interface Response {
  kind: "response";
  id: number;
  result?: unknown;
  error?: ErrorData;
}

export interface Event {
  kind: "event";
  name: string;
  data?: unknown;
}

export type Message = Request | Response | Event;

/**
 * Why a request failed:
 * - `refused` — the shell will not do it for this application (a name its manifest does not list,
 *   a session of another space, a link it may not open);
 * - `failed` — the shell tried, and it failed (the backend's error is the message);
 * - `unknown` — no such request;
 * - `invalid` — its params are not what the request takes;
 * - `closed` — the bridge went away before the answer came;
 * - `timeout` — no answer in time (the SDK's own connect).
 */
export type ErrorCode = "refused" | "failed" | "unknown" | "invalid" | "closed" | "timeout";

export interface ErrorData {
  code: ErrorCode;
  message: string;
  data?: unknown;
}

/** Where a surface is (an application's manifest, `[[surface]] point`). */
export type SurfacePoint =
  | "space"
  | "space.sidebar"
  | "space.tab"
  | "space.secondary"
  | "details.panel"
  | "settings.section"
  | "home.card"
  | "modal";

/** What a frame is: told at `hello`. */
export interface SurfaceInfo {
  /** The application's id. */
  app: string;
  point: SurfacePoint;
  /** The surface's id, unique among the application's of its point. */
  id: string;
  /** The interface space it belongs to, for a space's surfaces. */
  space: string | null;
  /** What it was opened with: a modal's or a secondary panel's params, a details panel's item. */
  params: unknown;
  /** Whether the shell sizes the frame to its content: the SDK reports its height. */
  autoHeight: boolean;
  /** Whether the application may ask the shell to open web pages (`openLink`). */
  links: boolean;
}

/** What the shell's look and state are, as far as a frame needs to follow them. */
export interface Environment {
  /** The theme in effect (the shell's choice resolved against the system's). */
  theme: "light" | "dark";
  /** The shell's design tokens, as their values for this theme and brand: `--name` → value. */
  tokens: Record<string, string>;
  locale: "en" | "zh-CN";
  /** The window is narrow (the shell's 960px break). */
  narrow: boolean;
  /** The frame can be seen: its place in the shell is on screen, and the page is visible. */
  visible: boolean;
  /** A dialog of the shell is over this frame: it takes no keys and no pointer until it closes. */
  modal: boolean;
}

/** A session, as the shell names one. */
export interface SessionRef {
  instance: string;
  sessionId: string;
}

/** An item a details panel draws: its `kind` is one of the application's (`<id>.<name>`). */
export interface DetailItem {
  kind: string;
  [key: string]: unknown;
}

/** A key pressed in a frame that is the shell's to act on (see `shellKey`). */
export interface KeyPress {
  key: string;
  code: string;
  meta: boolean;
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
}

export interface Welcome {
  surface: SurfaceInfo;
  env: Environment;
  /** The application's space's open session, if it has a space. */
  current: SessionRef | null;
}

/** What a frame asks of the shell: `params`, and what the response's `result` is. */
export interface FrameRequests {
  hello: { params: { sdk: string }; result: Welcome };
  /** A method: the application's own `app.<id>.*`, or one its `[interface] hub` lists. */
  call: { params: { method: string; params?: unknown }; result: unknown };
  /** A topic, the same way (`topic:<name>` in the hub). Values arrive as `topic` events. */
  subscribe: { params: { topic: string; params?: unknown }; result: { sub: number } };
  unsubscribe: { params: { sub: number }; result: null };
  "session.current": { params: Record<string, never>; result: SessionRef | null };
  /** Open a session of the application's own space, bringing that space to the front. */
  "session.open": { params: SessionRef; result: null };
  /** A new session, in the application's own space; opened there. */
  "session.create": { params: { scene?: string; options?: Record<string, unknown> }; result: SessionRef };
  "spaces.switch": { params: { space: string }; result: null };
  /** Open the right column, on one of the application's space's tabs. */
  "details.open": { params: { tab?: string }; result: null };
  /** Show an item of the application's in the right column's details. */
  "details.show": { params: { item: DetailItem }; result: null };
  /** Open the application's space's second side panel, with `params` for its frame. */
  "secondary.open": { params: { params?: unknown; title?: string }; result: null };
  "secondary.close": { params: Record<string, never>; result: null };
  /** Open Settings, on one of the application's sections. */
  "settings.open": { params: { section?: string }; result: null };
  "composer.insert": { params: { text: string }; result: null };
  /** A badge on the application's space entry; null or 0 clears it. */
  badge: { params: { value: string | number | null }; result: null };
  /** A line the shell shows, under the application's name. */
  notify: { params: { text: string; level?: "info" | "error" }; result: null };
  copy: { params: { text: string }; result: null };
  /** One of the application's modal surfaces; answers what it closed with (null if dismissed). */
  openModal: { params: { id: string; params?: unknown }; result: unknown };
  /** The shell's own dialog: true when the person confirmed. */
  confirm: { params: { text: string; title?: string; confirm?: string; danger?: boolean }; result: boolean };
  /** A web page, outside: the shell asks the person first (`[interface] links`). */
  openLink: { params: { url: string }; result: boolean };
  /** A file, uploaded by the shell for the next message's attachments. */
  upload: { params: { name: string; type: string; data: ArrayBuffer }; result: { path: string } };
}

export interface FrameEvents {
  /** An auto-height surface's content height, in CSS pixels. */
  resize: { height: number };
  key: KeyPress;
  /** A modal surface closes itself, with what it answers its opener. */
  close: { result?: unknown };
  /** An error the page did not catch: for the shell's diagnostics. */
  error: { message: string };
}

export interface ShellRequests {
  /** The heartbeat: a frame that stops answering is shown as failed. */
  ping: { params: Record<string, never>; result: "pong" };
}

export interface ShellEvents {
  env: Environment;
  topic: { sub: number; value: unknown };
  /** A subscription the shell ended (the topic went away). */
  "topic.end": { sub: number; reason: string };
  /** The application's space's open session changed. */
  session: { current: SessionRef | null };
  /** A modal, secondary panel or details panel was given new params. */
  params: { params: unknown };
}

/** The keys a frame hands to the shell: what the shell would act on, and Escape where it closes something. */
export function shellKey(press: KeyPress): "escape" | "new" | "sidebar" | "composer" | null {
  const mod = press.meta || press.ctrl;
  if (press.key === "Escape" && !mod) return "escape";
  if (mod && !press.alt && press.key.toLowerCase() === "k") return "new";
  if (mod && !press.alt && press.key === "\\") return "sidebar";
  if (mod && !press.alt && press.key === "/") return "composer";
  return null;
}

/** Whether a point's frame is drawn inside one of the shell's dialogs (Escape is the dialog's). */
export function inDialog(point: SurfacePoint): boolean {
  return point === "modal" || point === "settings.section";
}

/** How the shell narrows a call through a name `[interface] hub` lists: `open`, passed on as asked;
 *  `space`, about a space's sessions — only the application's own (`space` filled in when left
 *  out); `session`, about one session (`session_id`, `instance`) — only one in its own spaces. */
export type HubScope = "open" | "space" | "session";

/** Everything `[interface] hub` may list (`topic:<name>` for a topic), and how each is narrowed.
 *  An allowlist: the backend's `surface.rs` has the same table, refuses a manifest that lists
 *  anything else, and checks this one agrees. */
export const FRAME_HUB: Readonly<Record<string, HubScope>> = {
  "harnix.sessions": "space",
  "harnix.send": "session",
  "harnix.sessions.tasks": "session",
  "session.get": "session",
  "session.history": "session",
  "session.entries": "session",
  "session.interrupt": "session",
  "harnix.spaces.list": "open",
  "scene.list": "open",
  "topic:harnix.sessions.tasks": "session",
  "topic:harnix.spaces": "open",
};

/** Whether a frame of application `app` may call `name` (or subscribe to it, with `topic`): the
 *  application's own `app.<id>.*`, or a name its `[interface] hub` lists (`topic:<name>` for a
 *  topic) that `FRAME_HUB` has. The shell decides with this, then narrows the call by the name's
 *  scope; the backend's `surface.rs` states the same rule. */
export function admits(app: string, hub: readonly string[], name: string, topic: boolean): boolean {
  if (name.startsWith(`app.${app}.`)) return true;
  const listed = topic ? `topic:${name}` : name;
  return hub.includes(listed) && Object.hasOwn(FRAME_HUB, listed);
}
