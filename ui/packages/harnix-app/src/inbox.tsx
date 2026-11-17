/**
 * 待我处理 (harnix-app.md §3.3): one list of what waits on a person, across the sessions this device
 * sees and whatever else other packages say waits — a flow's confirm, its hold, an external task.
 *
 * The list knows no kind of thing by name. It defines the point `harnix-app.inbox`: a source of
 * items, each read from the source's own cache (`items()`, told of changes by `subscribe`) and
 * opened by it. This package is one source — the hub's `harnix.inbox` topic, every ask a session
 * holds: a permission, a plan's review, a question, an MCP server's form — and the flows are
 * another, from their own runs topic. The hub does not know the flows; the page puts the two
 * together.
 *
 * Where it shows: a block of a project's overview (only that project's), and an entry at the top of
 * the session and projects spaces' left column while anything waits, opening the whole list.
 */

import { useSyncExternalStore } from "react";
import { clip, Glyph, t, useT, type HostApi, type Registry } from "@harnix/react";
import type { Inbox as HubInboxValue, InboxRow, PromptEvent } from "@harnix/core";

export const INBOX = "harnix-app.inbox";

/** One thing waiting on a person. */
export interface InboxItem {
  /** Unique within its source. */
  id: string;
  /** What it waits for, as a word a test or a style can key on: `approval`, `plan`, `answer`,
   *  `elicitation` (the hub's); `confirm`, `held`, `external` (the flows'); a source's own. */
  kind: string;
  /** The same, in words ("待批准"). */
  label: string;
  /** What is waiting: the session's or the task's name. */
  title: string;
  /** One line more: the tool, the question, the stations. */
  detail?: string | null;
  /** The project it is in, when it is in one: the overview lists only its own. */
  project_root?: string | null;
  /** Whatever the source wants back in `open`. */
  data?: unknown;
}

/** A source of what waits (`harnix-app.inbox`, a `register` point). */
export interface InboxSource {
  /** `<owner>.<name>`: the hub's is `harnix-app.asks`, the flows' `flow.waiting`. */
  id: string;
  /** Its items now; read on each change, so answer from a cache. */
  items(): InboxItem[];
  /** Tell the list the items changed. Returns the unsubscribe. */
  subscribe(fn: () => void): () => void;
  /** Go to where it is answered: the session, the task. */
  open(item: InboxItem): void | Promise<void>;
}

/** An item with the source it came from. */
export interface InboxEntry {
  source: InboxSource & { owner: string };
  item: InboxItem;
}

/** Whether two spellings name one directory: a trailing `/` is not a difference. */
export function sameDir(a: string | null | undefined, b: string | null | undefined): boolean {
  const trim = (p: string) => p.replace(/\/+$/, "") || "/";
  return Boolean(a && b && trim(a) === trim(b));
}

/**
 * Every registered source's items, kept current: the sources from the registry, each followed by
 * its `subscribe` while it is registered, and the items read again whenever one says they changed.
 * A source whose `items` or `subscribe` throws contributes nothing and is counted as failed.
 *
 * One snapshot, `subscribe` / `getSnapshot`: a value that does not change unless an item did.
 */
export class Inbox {
  private entries: InboxEntry[] = [];
  private key = "";
  private readonly followed = new Map<string, { source: InboxSource & { owner: string }; off: () => void }>();
  private readonly listeners = new Set<() => void>();
  private offRegistry: (() => void) | null = null;

  constructor(private readonly r: Registry) {}

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  readonly getSnapshot = (): InboxEntry[] => this.entries;

  /** Follow the registry's sources from now on. */
  start(): void {
    if (this.offRegistry) return;
    this.offRegistry = this.r.subscribe(() => this.follow());
    this.follow();
  }

  stop(): void {
    this.offRegistry?.();
    this.offRegistry = null;
    for (const { off } of this.followed.values()) off();
    this.followed.clear();
  }

  /** What waits in one project; everything when `root` is not given. */
  of(root?: string | null): InboxEntry[] {
    return root === undefined ? this.entries : this.entries.filter((e) => sameDir(e.item.project_root, root));
  }

  /** Subscribe to the sources registered now, and let go of those withdrawn. */
  private follow(): void {
    const sources = this.r.list<InboxSource>(INBOX);
    const present = new Set(sources.map((s) => `${s.owner}/${s.id}`));
    for (const [key, { off }] of [...this.followed]) {
      if (present.has(key)) continue;
      off();
      this.followed.delete(key);
    }
    for (const source of sources) {
      const key = `${source.owner}/${source.id}`;
      const held = this.followed.get(key);
      if (held?.source === source) continue;
      held?.off();
      let off = () => undefined as void;
      try {
        off = source.subscribe(() => this.read());
      } catch (e) {
        this.r.fail(INBOX, source, e);
      }
      this.followed.set(key, { source, off });
    }
    this.read();
  }

  /** Read every source's items again; tell the listeners if anything differs. */
  private read(): void {
    const entries: InboxEntry[] = [];
    for (const { source } of this.followed.values()) {
      try {
        for (const item of source.items()) entries.push({ source, item });
      } catch (e) {
        this.r.fail(INBOX, source, e);
      }
    }
    const key = JSON.stringify(entries.map((e) => [e.source.id, e.item.id, e.item.kind, e.item.label, e.item.title, e.item.detail ?? null, e.item.project_root ?? null]));
    // The same items: the old snapshot stands, and nothing is drawn again. New `data` with the same
    // face is taken in quietly, so `open` gets the latest.
    if (key === this.key) {
      this.entries.splice(0, this.entries.length, ...entries);
      return;
    }
    this.key = key;
    this.entries = entries;
    for (const fn of [...this.listeners]) fn();
  }
}

/** The inbox's items, in a component. */
export function useInbox(inbox: Inbox, root?: string | null): InboxEntry[] {
  useSyncExternalStore(inbox.subscribe, inbox.getSnapshot);
  return inbox.of(root);
}

/* ── the hub's asks ───────────────────────────────────────────────────── */

/** What an ask waits for, as an item's kind: the row's `waiting`, an MCP server's form apart. */
export function askKind(row: Pick<InboxRow, "waiting" | "prompt">): string {
  return row.prompt?.prompt_type === "mcp_elicitation" ? "elicitation" : row.waiting;
}

/** One line about an ask: the tool a permission is for, the question, the server and what it asks. */
export function askDetail(prompt: PromptEvent | null | undefined): string | null {
  if (!prompt) return null;
  const line = (text: unknown) => (typeof text === "string" && text.trim() ? clip(text.trim().replace(/\s+/g, " "), 120) : null);
  switch (prompt.prompt_type) {
    case "question":
      return line(prompt.question ?? prompt.message);
    case "mcp_elicitation": {
      const message = line(prompt.message);
      return prompt.server ? (message ? `${prompt.server} · ${message}` : prompt.server) : message;
    }
    default:
      return line(prompt.tool_question) ?? line(prompt.tool_name);
  }
}

/** A row of `harnix.inbox` as an item. */
export function askItem(row: InboxRow, index: number): InboxItem {
  const kind = askKind(row);
  return {
    id: `${row.instance}/${row.session_id}/${row.prompt?.prompt_id ?? index}`,
    kind,
    label: t(`inbox.kind.${kind}`),
    title: row.name || t("sidebar.untitled"),
    detail: kind === "plan" ? null : askDetail(row.prompt),
    project_root: row.project_root ?? null,
    data: row,
  };
}

/**
 * The hub's source: the `harnix.inbox` topic, followed while the list is. An item opens its
 * session in the space it belongs to, where its card is answered.
 */
export function hubAsks(host: HostApi): InboxSource {
  let rows: InboxRow[] = [];
  const listeners = new Set<() => void>();
  let off: (() => void) | null = null;
  return {
    id: "harnix-app.asks",
    items: () => rows.map(askItem),
    subscribe(fn) {
      listeners.add(fn);
      if (!off) {
        off = host.subscribe("harnix.inbox", {}, (value) => {
          rows = (value as HubInboxValue | null)?.items ?? [];
          for (const l of [...listeners]) l();
        });
      }
      return () => {
        listeners.delete(fn);
        if (!listeners.size) {
          off?.();
          off = null;
        }
      };
    },
    open: (item) => {
      const row = item.data as InboxRow;
      return host.session.open({ instance: row.instance, sessionId: row.session_id }, { switchSpace: true });
    },
  };
}

/* ── drawing it ───────────────────────────────────────────────────────── */

function basename(path: string): string {
  return path.replace(/\/+$/, "").split("/").pop() || path;
}

/**
 * The list: one row per item, what it waits for, what is waiting and a line more; the project
 * when the list is not one project's. A row clicked opens it (and `onOpen` is told, a dialog closes).
 */
export function InboxList({ r, inbox, root, limit, onOpen }: { r: Registry; inbox: Inbox; root?: string | null; limit?: number; onOpen?: () => void }) {
  const tt = useT();
  const entries = useInbox(inbox, root);
  const shown = limit ? entries.slice(0, limit) : entries;
  if (!entries.length) return <div className="hx-inbox empty" data-testid="inbox-empty">{tt("inbox.empty")}</div>;
  const open = (entry: InboxEntry) => {
    onOpen?.();
    void Promise.resolve()
      .then(() => entry.source.open(entry.item))
      .catch((e: unknown) => r.fail(INBOX, entry.source, e));
  };
  return (
    <div className="hx-inbox" data-testid="inbox-list">
      {shown.map((entry) => (
        <button
          type="button"
          key={`${entry.source.owner}/${entry.source.id}/${entry.item.id}`}
          className="hx-inbox-row"
          data-testid="inbox-item"
          data-kind={entry.item.kind}
          data-source={entry.source.id}
          data-item={entry.item.id}
          title={entry.item.detail ? `${entry.item.title} · ${entry.item.detail}` : entry.item.title}
          onClick={() => open(entry)}
        >
          <span className="hx-inbox-kind" data-kind={entry.item.kind}>
            {entry.item.label}
          </span>
          <span className="hx-inbox-text">
            <span className="hx-inbox-title">{entry.item.title}</span>
            {entry.item.detail ? <span className="hx-inbox-detail">{entry.item.detail}</span> : null}
          </span>
          {root === undefined && entry.item.project_root ? <span className="hx-inbox-project">{basename(entry.item.project_root)}</span> : null}
        </button>
      ))}
      {shown.length < entries.length ? <div className="hx-inbox-more">{tt("inbox.more", { count: entries.length - shown.length })}</div> : null}
    </div>
  );
}

/** The whole list, in the shell's dialog; a row opened closes it. */
export function inboxDialog(host: HostApi, r: Registry, inbox: Inbox): Promise<null> {
  return host.dialogs.open<never>((close) => <InboxList r={r} inbox={inbox} onOpen={() => close(null)} />, { title: t("inbox.title"), wide: true });
}

/** At the top of the session and projects spaces' left column, while anything waits: how many, opening the list. */
export function InboxEntryButton({ host, r, inbox }: { host: HostApi; r: Registry; inbox: Inbox }) {
  const tt = useT();
  const entries = useInbox(inbox);
  if (!entries.length) return null;
  return (
    <button type="button" className="hx-inbox-entry" data-testid="inbox-open" data-count={entries.length} onClick={() => void inboxDialog(host, r, inbox)}>
      <Glyph name="alert" className="glyph" />
      <span className="lbl">{tt("inbox.title")}</span>
      <span className="c">{entries.length}</span>
    </button>
  );
}
