/**
 * 一轮改了哪些文件：中枢在一轮结束时记进会话日志的那条条目（`harnix.changes` / `turn`）。
 *
 * 引擎只在工具结果旁边报一次写了哪些文件（Write、Edit、NotebookEdit），transcript 里没有 diff；中枢把一轮的这些
 * 合并成一条条目，刷新、重开会话都还在（docs/harnix-app.md）。形状是 Harnix 的，这里是它唯一的读法：
 * 改动卡与改动页签都从这里拿，不各自去读 payload。
 */

/** 条目的命名空间与事件名。 */
export const CHANGES_NS = "harnix.changes";
export const CHANGES_EVENT = "turn";
/**
 * 回退撤掉了几轮：中枢记一条 `rewind`（`{rewind_id, turn_ids}`），日志只追加不删。它之前、这几轮的 `turn` 条目
 * 中枢列条目（`engine.session.entries`）时已经去掉，界面不必再筛；这条本身不画。
 */
export const CHANGES_REWIND = "rewind";

/** 一个文件在这一轮里的改动（同一文件写了几次合成一行）。 */
export interface FileChange {
  /** 绝对路径，引擎报的写入目标。 */
  path: string;
  /** `create`：这一轮之前没有它。 */
  op: "create" | "modify";
  added: number;
  removed: number;
  /** 写完之后的大小，引擎量了才有。 */
  bytes?: number;
  /** unified diff（3 行上下文；写了几次就几段，按次序）。没有时 `omitted` 说为什么。 */
  diff?: string;
  /** diff 在 hunk 边界被截断了。 */
  truncated?: boolean;
  /** `too_large`、`binary`、`redacted`。 */
  omitted?: string;
}

/** 一轮的改动。 */
export interface TurnChanges {
  turnId: string;
  files: FileChange[];
  added: number;
  removed: number;
  /** 超出条目上限、没列出来的文件数。 */
  more: number;
}

const num = (v: unknown): number => (typeof v === "number" && Number.isFinite(v) ? v : 0);

/** 一条会话条目若是一轮的改动，读出来；别的条目、读不懂的是 null。 */
export function parseTurnChanges(entry: { ns?: unknown; event?: unknown; payload?: unknown } | null | undefined): TurnChanges | null {
  if (!entry || entry.ns !== CHANGES_NS || entry.event !== CHANGES_EVENT) return null;
  const p = (entry.payload && typeof entry.payload === "object" ? entry.payload : {}) as Record<string, unknown>;
  if (typeof p.turn_id !== "string" || !Array.isArray(p.files)) return null;
  const files: FileChange[] = [];
  for (const raw of p.files as unknown[]) {
    const f = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
    if (typeof f.path !== "string" || !f.path) continue;
    files.push({
      path: f.path,
      op: f.op === "create" ? "create" : "modify",
      added: num(f.added),
      removed: num(f.removed),
      ...(typeof f.bytes === "number" ? { bytes: f.bytes } : {}),
      ...(typeof f.diff === "string" ? { diff: f.diff } : {}),
      ...(f.truncated === true ? { truncated: true } : {}),
      ...(typeof f.omitted === "string" ? { omitted: f.omitted } : {}),
    });
  }
  return {
    turnId: p.turn_id,
    files,
    added: typeof p.added === "number" ? p.added : files.reduce((n, f) => n + f.added, 0),
    removed: typeof p.removed === "number" ? p.removed : files.reduce((n, f) => n + f.removed, 0),
    more: num(p.more),
  };
}

/** `path` 相对于项目根（在根里时）；不在根里的原样给回。 */
export function projectRelative(path: string, root: string | null | undefined): string {
  if (!root) return path;
  const base = root.replace(/\/+$/, "");
  return path.startsWith(`${base}/`) ? path.slice(base.length + 1) : path;
}
