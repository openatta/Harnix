/**
 * The right column's pages about the conversation's project (harnix-ext.md §22.7): the files tab
 * (a read-only tree of the session's project), the preview (text and code, Markdown, images —
 * `harnix-resource://file/**`, as the fallback anything specific beats), and the changes tab (which
 * files each turn changed, each with its diff), with the changes card a settled turn leaves in the
 * conversation and the "open in preview" door on a file tool's row.
 *
 * The files come from `harnix-app.files.list` / `files.read`, fenced in the session's project; the
 * changes from the session's log (`harnix.changes` entries, read by `@harnix/core`), so they are
 * there after a refresh. A file is addressed relative to the project root.
 */

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { CHANGES_NS, parseTurnChanges, projectRelative, type Block, type BlockOf, type FileChange, type SessionEntry, type TurnChanges } from "@harnix/core";
import {
  Glyph,
  UnifiedDiffView,
  fileSize,
  markdown,
  parseResource,
  resourceAddress,
  t,
  useT,
  useTabInfo,
  type ActionTarget,
  type HostApi,
  type Registry,
  type HostSessionRef as SessionRef,
} from "@harnix/react";

import type { AppStore } from "./store.ts";
import "./sidebar.css";

export const FILES_KIND = "files";
export const PREVIEW_KIND = "preview";
export const CHANGES_KIND = "changes";
/** The addresses the preview claims: a file of the session's project. */
export const FILE_PATTERN = "harnix-resource://file/**";

/** What the preview leaves to something else: office documents and PDFs are not drawn here. */
const UNSUPPORTED = new Set(["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "key", "pages", "numbers"]);
/** The most lines a text preview draws: past it, it says so. */
const MAX_LINES = 20_000;

interface Listing {
  root: string;
  path: string;
  entries: { name: string; path: string; dir: boolean; link?: boolean; size?: number }[];
  hidden: number;
  truncated: boolean;
}

type Read =
  | { kind: "text"; path: string; size: number; text: string }
  | { kind: "image"; path: string; size: number; mime: string; data: string }
  | { kind: "binary"; path: string; size: number }
  | { kind: "too_large"; path: string; size: number };

/** Where a call about a session claims it is: the backend space the front space lists. */
export function claimFor(r: Registry, host: HostApi): string {
  const front = host.spaces.current();
  return r.at("space").find((c) => c.id === front)?.sessions ?? front;
}

function sessionParams(r: Registry, host: HostApi, session: SessionRef): Record<string, unknown> {
  return { instance: session.instance, session_id: session.sessionId, space: claimFor(r, host) };
}

function message(e: unknown): string {
  return e instanceof Error ? e.message : String((e as { message?: unknown })?.message ?? e);
}

/** The session's project root, as the list has it (or the open session's own record). */
function rootOf(host: HostApi, store: AppStore, session: SessionRef | null): string | null {
  if (!session) return null;
  const listed = store.row(session.instance, session.sessionId)?.project_root;
  if (typeof listed === "string" && listed) return listed;
  const open = host.session.current();
  const context = open && open.sessionId === session.sessionId ? host.session.context()?.session?.project_root : null;
  return typeof context === "string" && context ? context : null;
}

/* ── which file a tool call is about ──────────────────────────────────── */

/**
 * The file a tool call names, relative to the session's project, and where in it to look: a read's
 * `offset`, or the text an edit put in. Null when it names none, or one outside the project.
 */
export function fileOfCall(input: unknown, root: string | null): { path: string; params: Record<string, unknown> } | null {
  if (!input || typeof input !== "object") return null;
  const record = input as Record<string, unknown>;
  const named = typeof record.file_path === "string" ? record.file_path : typeof record.notebook_path === "string" ? record.notebook_path : null;
  if (!named) return null;
  const path = named.startsWith("/") ? projectRelative(named, root) : named;
  if (!path || path.startsWith("/")) return null;
  const params: Record<string, unknown> = {};
  if (typeof record.offset === "number" && record.offset > 0) params.line = record.offset;
  else if (typeof record.new_string === "string" && record.new_string) params.text = record.new_string;
  else if (Array.isArray(record.edits)) {
    const first = (record.edits as { new_string?: unknown }[]).find((e) => typeof e?.new_string === "string" && e.new_string);
    if (first) params.text = first.new_string;
  }
  return { path, params };
}

/* ── registration ─────────────────────────────────────────────────────── */

export function registerSidebar(r: Registry, host: HostApi, store: AppStore, owner: string): void {
  r.register(
    "sidebar.tab",
    {
      id: "harnix-app.files",
      kind: FILES_KIND,
      priority: "builtin",
      title: () => t("files.title"),
      guide: [{ id: "files", title: () => t("files.title"), description: () => t("files.guide"), icon: "folder", order: 30 }],
      // The tree's expansion survives switching tabs and sessions.
      keepMounted: true,
      render: () => <FilesTab r={r} host={host} />,
    },
    owner,
  );
  r.register(
    "sidebar.tab",
    {
      id: "harnix-app.preview",
      kind: PREVIEW_KIND,
      patterns: [FILE_PATTERN],
      priority: "fallback",
      title: (address) => {
        const path = address ? (parseResource(address)?.path ?? "") : "";
        return path.split("/").pop() || t("preview.title");
      },
      render: () => <PreviewTab r={r} host={host} />,
    },
    owner,
  );
  r.register(
    "sidebar.tab",
    {
      id: "harnix-app.changes",
      kind: CHANGES_KIND,
      priority: "builtin",
      title: () => t("changes.title"),
      guide: [{ id: "changes", title: () => t("changes.title"), description: () => t("changes.guide"), icon: "edit", order: 40 }],
      render: () => <ChangesTab r={r} host={host} store={store} />,
    },
    owner,
  );

  // The card a turn that wrote files leaves after it in the conversation.
  r.register(
    "flow.block",
    {
      id: "harnix-app.changes",
      match: (block: Block) => block.kind === "custom" && block.type === "entry" && parseTurnChanges((block as BlockOf<"custom">).data as SessionEntry) != null,
      render: (block: Block) => {
        const entry = (block as BlockOf<"custom">).data as SessionEntry;
        return <ChangesCard host={host} store={store} entry={entry} changes={parseTurnChanges(entry)!} />;
      },
    },
    owner,
  );

  // A file tool's path opens the file, at the line it is about.
  const fileOf = (target: ActionTarget) => (target.kind === "tool" ? fileOfCall(target.input, rootOf(host, store, target.session)) : null);
  r.register(
    "session.action",
    {
      id: "harnix-app.open-file",
      at: ["arg", "tool"],
      icon: "file",
      label: () => t("preview.open"),
      match: (target) => fileOf(target) != null,
      run(target) {
        const file = fileOf(target);
        if (file) host.sidebar.openResource(resourceAddress("file", file.path), { params: file.params });
      },
    },
    owner,
  );
}

/* ── the files tab ────────────────────────────────────────────────────── */

function FilesTab({ r, host }: { r: Registry; host: HostApi }) {
  const tt = useT();
  const { tab } = useTabInfo();
  const session = tab.session;
  const [all, setAll] = useState(false);
  const [open, setOpen] = useState<Set<string>>(() => new Set());
  const [dirs, setDirs] = useState<Map<string, Listing | { error: string }>>(() => new Map());
  const [generation, setGeneration] = useState(0);

  const load = (path: string) => {
    if (!session) return;
    host
      .call<Listing>("harnix-app.files.list", { ...sessionParams(r, host, session), path, all })
      .then((listing) => setDirs((m) => new Map(m).set(path, listing)))
      .catch((e: unknown) => setDirs((m) => new Map(m).set(path, { error: message(e) })));
  };
  // Again from the root when the session, the toggle or a refresh changes: what was open is read again.
  useEffect(() => {
    setDirs(new Map());
    load("");
    for (const path of open) load(path);
  }, [session?.instance, session?.sessionId, all, generation]);

  const toggle = (path: string) => {
    setOpen((s) => {
      const next = new Set(s);
      if (next.has(path)) next.delete(path);
      else {
        next.add(path);
        if (!dirs.has(path)) load(path);
      }
      return next;
    });
  };

  const root = dirs.get("");
  if (!session) return <div className="hx-files empty">{tt("files.noProject")}</div>;
  if (root && "error" in root) {
    return (
      <div className="hx-files" data-testid="files-tab" data-state="failed">
        <div className="empty">{/no project/.test(root.error) ? tt("files.noProject") : tt("files.failed", { error: root.error })}</div>
      </div>
    );
  }
  return (
    <div className="hx-files" data-testid="files-tab" data-state={root ? "ready" : "loading"}>
      <div className="hx-files-head">
        <Glyph name="folder" className="glyph" />
        <span className="name" title={root?.root}>
          {root?.root.split("/").pop() || tt("files.title")}
        </span>
        <button type="button" className="btn sm ghost" data-testid="files-toggle-ignored" aria-pressed={all} onClick={() => setAll((v) => !v)}>
          {all ? tt("files.hideIgnored") : tt("files.showIgnored", { count: root?.hidden ?? 0 })}
        </button>
        <button type="button" className="ib" data-testid="files-refresh" title={tt("files.refresh")} aria-label={tt("files.refresh")} onClick={() => setGeneration((g) => g + 1)}>
          <Glyph name="refresh" className="glyph" />
        </button>
      </div>
      {root ? (
        <div className="hx-files-tree" role="tree">
          <Entries listing={root} depth={0} dirs={dirs} open={open} toggle={toggle} onFile={(path) => tab.actions.openResource(resourceAddress("file", path))} />
        </div>
      ) : (
        <div className="empty">{tt("common.loading")}</div>
      )}
    </div>
  );
}

function Entries({
  listing,
  depth,
  dirs,
  open,
  toggle,
  onFile,
}: {
  listing: Listing | { error: string } | undefined;
  depth: number;
  dirs: Map<string, Listing | { error: string }>;
  open: Set<string>;
  toggle(path: string): void;
  onFile(path: string): void;
}) {
  const tt = useT();
  const pad = { paddingInlineStart: `${8 + depth * 14}px` };
  if (!listing) return <div className="hx-files-note" style={pad}>{tt("common.loading")}</div>;
  if ("error" in listing) return <div className="hx-files-note bad" style={pad}>{tt("files.failed", { error: listing.error })}</div>;
  if (!listing.entries.length) return <div className="hx-files-note" style={pad}>{tt("files.empty")}</div>;
  return (
    <>
      {listing.entries.map((entry) => {
        const expanded = entry.dir && open.has(entry.path);
        return (
          <div key={entry.path} role="none">
            <button
              type="button"
              role="treeitem"
              className="hx-files-row"
              style={pad}
              data-testid="files-row"
              data-path={entry.path}
              data-dir={entry.dir ? "1" : undefined}
              data-state={entry.dir ? (expanded ? "open" : "closed") : undefined}
              aria-expanded={entry.dir ? expanded : undefined}
              title={entry.path}
              onClick={() => (entry.dir ? toggle(entry.path) : onFile(entry.path))}
            >
              <Glyph name={entry.dir ? (expanded ? "chevronDown" : "chevronRight") : "file"} className="glyph" />
              <span className="name">{entry.name}</span>
              {entry.link ? <Glyph name="link" className="glyph faint" /> : null}
              {!entry.dir && entry.size != null ? <span className="size">{fileSize(entry.size)}</span> : null}
            </button>
            {expanded ? <Entries listing={dirs.get(entry.path)} depth={depth + 1} dirs={dirs} open={open} toggle={toggle} onFile={onFile} /> : null}
          </div>
        );
      })}
      {listing.truncated ? <div className="hx-files-note" style={pad}>{tt("files.truncated", { count: listing.entries.length })}</div> : null}
    </>
  );
}

/* ── the preview ──────────────────────────────────────────────────────── */

/** The line a navigation asks for: a number, or the first line the given text starts on. */
export function lineOf(params: Record<string, unknown>, text: string | null): number | null {
  if (typeof params.line === "number" && params.line > 0) return Math.floor(params.line);
  if (typeof params.text === "string" && params.text && text != null) {
    const at = text.indexOf(params.text);
    if (at >= 0) return text.slice(0, at).split("\n").length;
  }
  return null;
}

function PreviewTab({ r, host }: { r: Registry; host: HostApi }) {
  const tt = useT();
  const { tab } = useTabInfo();
  const session = tab.session;
  const path = parseResource(tab.address ?? "")?.path ?? "";
  const ext = /\.([^./]+)$/.exec(path)?.[1]?.toLowerCase() ?? "";
  const unsupported = UNSUPPORTED.has(ext);
  const [read, setRead] = useState<Read | { error: string } | null>(null);
  const [reload, setReload] = useState(0);
  const [source, setSource] = useState(false);
  const body = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!session || unsupported) return;
    let live = true;
    setRead(null);
    host
      .call<Read>("harnix-app.files.read", { ...sessionParams(r, host, session), path })
      .then((answer) => live && setRead(answer))
      .catch((e: unknown) => live && setRead({ error: message(e) }));
    return () => {
      live = false;
    };
  }, [session?.instance, session?.sessionId, path, reload, unsupported]);

  const text = read && "kind" in read && read.kind === "text" ? read.text : null;
  const line = lineOf(tab.navigation.params, text);
  const markdownFile = ext === "md" || ext === "markdown";
  // A navigation to a line shows the source: a rendered page has no lines to land on.
  const showSource = !markdownFile || source || line != null;

  // Each navigation lands on its line again, the first one included.
  useEffect(() => {
    if (line == null || text == null) return;
    const row = body.current?.querySelector(`[data-line="${line}"]`);
    if (row && "scrollIntoView" in row) (row as HTMLElement).scrollIntoView({ block: "center" });
  }, [line, text, tab.navigation.revision]);

  let content: ReactNode;
  if (unsupported) content = <div className="empty" data-testid="preview-message">{tt("preview.unsupported", { ext })}</div>;
  else if (!read) content = <div className="empty">{tt("common.loading")}</div>;
  else if ("error" in read) content = <div className="empty bad" data-testid="preview-message">{tt("preview.failed", { error: read.error })}</div>;
  else if (read.kind === "image") content = <img className="hx-preview-image" data-testid="preview-image" alt={path} src={`data:${read.mime};base64,${read.data}`} />;
  else if (read.kind === "binary") content = <div className="empty" data-testid="preview-message">{tt("preview.binary", { size: fileSize(read.size) })}</div>;
  else if (read.kind === "too_large") content = <div className="empty" data-testid="preview-message">{tt("preview.tooLarge", { size: fileSize(read.size) })}</div>;
  else if (!showSource) {
    const untrusted = host.session.context()?.trust === "extension";
    content = <div className="hx-md hx-preview-md" data-testid="preview-markdown" dangerouslySetInnerHTML={{ __html: markdown(read.text, { untrusted }) }} />;
  } else content = <Code text={read.text} line={line} revision={tab.navigation.revision} />;

  return (
    <div className="hx-preview" data-testid="preview-tab" data-path={path} data-line={line ?? undefined}>
      <div className="hx-files-head">
        <Glyph name="file" className="glyph" />
        <span className="name" title={path}>
          {path}
        </span>
        {markdownFile && text != null ? (
          <button type="button" className="btn sm ghost" data-testid="preview-toggle-source" aria-pressed={showSource} disabled={line != null} onClick={() => setSource((v) => !v)}>
            {showSource ? tt("preview.rendered") : tt("preview.source")}
          </button>
        ) : null}
        <button type="button" className="ib" data-testid="preview-reload" title={tt("preview.reload")} aria-label={tt("preview.reload")} onClick={() => setReload((n) => n + 1)}>
          <Glyph name="refresh" className="glyph" />
        </button>
      </div>
      <div className="hx-preview-body" ref={body}>
        {content}
      </div>
    </div>
  );
}

/** Text with its line numbers; `line`, when given, is marked. No highlighter: the code as written. */
function Code({ text, line, revision }: { text: string; line: number | null; revision: number }) {
  const tt = useT();
  const lines = useMemo(() => text.replace(/\n$/, "").split("\n"), [text]);
  const shown = lines.length > MAX_LINES ? lines.slice(0, MAX_LINES) : lines;
  return (
    <div className="hx-preview-code" data-testid="preview-code" data-lines={lines.length} data-revision={revision}>
      {shown.map((l, i) => (
        <div key={i} className="row" data-line={i + 1} data-state={line === i + 1 ? "hit" : undefined} data-testid={line === i + 1 ? "preview-hit" : undefined}>
          <span className="ln">{i + 1}</span>
          <code>{l || " "}</code>
        </div>
      ))}
      {shown.length < lines.length ? <div className="hx-files-note">{tt("preview.cut", { count: MAX_LINES })}</div> : null}
    </div>
  );
}

/* ── the changes ──────────────────────────────────────────────────────── */

/** A turn's time as its card says it: the clock, today; the date with it, otherwise. */
function when(ts: string | undefined): string {
  if (!ts) return "";
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return "";
  const hm = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  return d.toDateString() === new Date().toDateString() ? hm : `${d.getMonth() + 1}/${d.getDate()} ${hm}`;
}

function Counts({ added, removed }: { added: number; removed: number }) {
  return (
    <span className="hx-changes-counts">
      <span className="add">+{added}</span> <span className="del">−{removed}</span>
    </span>
  );
}

function ChangesCard({ host, store, entry, changes }: { host: HostApi; store: AppStore; entry: SessionEntry; changes: TurnChanges }) {
  const tt = useT();
  const current = host.session.current();
  const root = rootOf(host, store, current);
  const count = changes.files.length + changes.more;
  const names = changes.files.slice(0, 3).map((f) => projectRelative(f.path, root).split("/").pop());
  return (
    <div className="hx-blk hx-changes-card" data-testid="changes-card" data-turn={changes.turnId} data-reveal={`changes:${changes.turnId}`}>
      <button
        type="button"
        data-testid="changes-card-open"
        title={tt("changes.cardHint")}
        onClick={() => host.sidebar.openTab(CHANGES_KIND, { params: { turn: changes.turnId } })}
      >
        <Glyph name="edit" className="glyph" />
        <b>{tt("changes.card", { count })}</b>
        <Counts added={changes.added} removed={changes.removed} />
        <span className="files">
          {names.join(", ")}
          {count > names.length ? " …" : ""}
        </span>
        {entry.ts ? <span className="at">{when(entry.ts)}</span> : null}
      </button>
    </div>
  );
}

function ChangesTab({ r, host, store }: { r: Registry; host: HostApi; store: AppStore }) {
  const tt = useT();
  const { tab } = useTabInfo();
  const session = tab.session;
  const wanted = typeof tab.navigation.params.turn === "string" ? tab.navigation.params.turn : null;
  const [showAll, setShowAll] = useState(false);
  const [turns, setTurns] = useState<{ changes: TurnChanges; ts?: string }[] | { error: string } | null>(null);
  const [generation, setGeneration] = useState(0);

  useEffect(() => setShowAll(false), [wanted, tab.navigation.revision]);
  // A turn that ends in this session may have changed more.
  useEffect(() => {
    if (!session) return;
    return host.events.onTurnSettled((e) => {
      if ((e as { session_id?: string }).session_id === session.sessionId) setGeneration((g) => g + 1);
    });
  }, [host, session?.instance, session?.sessionId]);
  useEffect(() => {
    if (!session) return;
    let live = true;
    host
      .call<{ entries?: SessionEntry[] }>("engine.session.entries", { ...sessionParams(r, host, session), namespaces: [CHANGES_NS] })
      .then((listed) => {
        if (!live) return;
        const read = (listed.entries ?? []).flatMap((entry) => {
          const changes = parseTurnChanges(entry);
          return changes ? [{ changes, ts: entry.ts }] : [];
        });
        setTurns(read.reverse());
      })
      .catch((e: unknown) => live && setTurns({ error: message(e) }));
    return () => {
      live = false;
    };
  }, [session?.instance, session?.sessionId, tab.navigation.revision, generation]);

  const root = rootOf(host, store, session);
  if (!turns) return <div className="hx-changes empty">{tt("common.loading")}</div>;
  if ("error" in turns) return <div className="hx-changes empty bad">{tt("files.failed", { error: turns.error })}</div>;
  const shown = wanted && !showAll ? turns.filter((x) => x.changes.turnId === wanted) : turns;
  return (
    <div className="hx-changes" data-testid="changes-tab" data-turn={wanted && !showAll ? wanted : undefined}>
      {!turns.length ? <div className="empty">{tt("changes.none")}</div> : null}
      {turns.length && !shown.length ? <div className="empty">{tt("changes.noTurn")}</div> : null}
      {shown.map(({ changes, ts }) => (
        <section key={changes.turnId} className="hx-changes-turn" data-testid="changes-turn" data-turn={changes.turnId}>
          <div className="hx-files-head">
            <b>{tt("changes.card", { count: changes.files.length + changes.more })}</b>
            <Counts added={changes.added} removed={changes.removed} />
            {ts ? <span className="at">{tt("changes.turnAt", { time: when(ts) })}</span> : null}
          </div>
          {changes.files.map((file, i) => (
            <ChangedFile
              key={file.path}
              file={file}
              path={projectRelative(file.path, root)}
              initiallyOpen={i < 5}
              onOpen={(path) => tab.actions.openResource(resourceAddress("file", path))}
            />
          ))}
          {changes.more ? <div className="hx-files-note">{tt("changes.more", { count: changes.more })}</div> : null}
        </section>
      ))}
      {wanted && !showAll && turns.length > shown.length ? (
        <button type="button" className="btn sm ghost" data-testid="changes-show-all" onClick={() => setShowAll(true)}>
          {tt("changes.showAll")}
        </button>
      ) : null}
    </div>
  );
}

function ChangedFile({ file, path, initiallyOpen, onOpen }: { file: FileChange; path: string; initiallyOpen: boolean; onOpen(path: string): void }) {
  const tt = useT();
  const [open, setOpen] = useState(initiallyOpen);
  const inProject = !path.startsWith("/");
  return (
    <div className="hx-changes-file" data-testid="changes-file" data-path={path} data-state={open ? "open" : "closed"}>
      <div className="row">
        <button type="button" className="toggle" aria-expanded={open} data-testid="changes-file-toggle" onClick={() => setOpen((v) => !v)}>
          <Glyph name={open ? "chevronDown" : "chevronRight"} className="glyph" />
          <span className="name" title={file.path}>
            {path}
          </span>
          {file.op === "create" ? <span className="hx-tag">{tt("changes.created")}</span> : null}
          <Counts added={file.added} removed={file.removed} />
        </button>
        {inProject ? (
          <button type="button" className="btn sm ghost" data-testid="changes-open-file" onClick={() => onOpen(path)}>
            {tt("changes.openFile")}
          </button>
        ) : null}
      </div>
      {open ? (
        file.diff ? (
          <>
            <UnifiedDiffView path={path} diff={file.diff} testId="changes-diff" />
            {file.truncated ? <div className="hx-files-note">{tt("changes.truncated")}</div> : null}
          </>
        ) : (
          <div className="hx-files-note">{tt(`changes.omitted.${file.omitted ?? "too_large"}`)}</div>
        )
      ) : null}
    </div>
  );
}
