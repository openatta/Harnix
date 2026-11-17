/**
 * 对一轮回答的反馈：中枢记进会话日志的条目（`harnix.feedback` / `rating`，载荷是生成的 `Feedback`）。
 *
 * 评、改、撤都是再记一条，同一轮（与同一次调用）后来的盖过先前的，`rating: null` 是撤回；早先的仍在日志里。
 * 条目只给人看，模型看不见；刷新、重开、分叉之后都还在（docs/harnix-app.md）。界面只从这里读它，不各自去读 payload。
 */

import { FEEDBACK_EVENT, FEEDBACK_NS, type Feedback, type FeedbackCategory } from "./generated/contract.ts";

/** 对话里给的类别，按这个次序列出。 */
export const FEEDBACK_CATEGORIES: readonly FeedbackCategory[] = ["inaccurate", "unhelpful", "unsafe", "slow", "other"];

type EntryLike = { ns?: unknown; event?: unknown; payload?: unknown } | null | undefined;

/** 一条会话条目是反馈时读出来；别的条目、读不懂的是 null。 */
export function parseFeedback(entry: EntryLike): Feedback | null {
  if (!entry || entry.ns !== FEEDBACK_NS || entry.event !== FEEDBACK_EVENT) return null;
  const p = (entry.payload && typeof entry.payload === "object" ? entry.payload : {}) as Record<string, unknown>;
  if (typeof p.turn_id !== "string" || !p.turn_id) return null;
  const rating = p.rating === "up" || p.rating === "down" ? p.rating : null;
  return {
    turn_id: p.turn_id,
    ...(typeof p.call_id === "string" ? { call_id: p.call_id } : {}),
    rating,
    ...(rating && FEEDBACK_CATEGORIES.includes(p.category as FeedbackCategory) ? { category: p.category as FeedbackCategory } : {}),
    ...(rating && typeof p.note === "string" && p.note ? { note: p.note } : {}),
    at: typeof p.at === "number" ? p.at : 0,
    by: typeof p.by === "string" ? p.by : "",
  };
}

/** 条目是不是反馈（对话流里不画它，状态画在那一轮的回答下面）。 */
export function isFeedbackEntry(entry: EntryLike): boolean {
  return Boolean(entry && entry.ns === FEEDBACK_NS && entry.event === FEEDBACK_EVENT);
}

/**
 * 每一轮现在的反馈（对整轮回答的，不含只评某一次调用的），`entries` 按日志次序、最早的在前：
 * 后来的盖过先前的，撤回了的不在里面。
 */
export function standingFeedback(entries: Iterable<EntryLike>): Map<string, Feedback> {
  const standing = new Map<string, Feedback>();
  for (const entry of entries) {
    const feedback = parseFeedback(entry);
    if (!feedback || feedback.call_id != null) continue;
    if (feedback.rating) standing.set(feedback.turn_id, feedback);
    else standing.delete(feedback.turn_id);
  }
  return standing;
}
