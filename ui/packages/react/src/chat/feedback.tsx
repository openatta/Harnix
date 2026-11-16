/**
 * 对一轮回答的反馈：动作行里的赞与踩，和点了之后的小对话框（类别、备注；改、撤）。
 *
 * 反馈是会话日志里的条目（`@harnix/core` 的 `standingFeedback` 读它），记在本机、模型看不见；
 * 有没有、往哪里送是应用的事（持 `feedback.read` 的扩展），这里只记（docs/harnix-app.md）。
 * 别的设备记的、改的，经条目的推送到这里，按钮随之变。
 *
 * 对话框不是模态的：它是一个层（`layers.ts`），Esc 关掉、点外面关掉，下面的页面照常可用。
 */

import { useLayoutEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type RefObject } from "react";
import { FEEDBACK_CATEGORIES, FEEDBACK_NOTE_MAX_CHARS, type Feedback, type FeedbackCategory, type Rating } from "@harnix/core/protocol";
import { useStrings } from "../context";
import { useLayer } from "../layers";
import { Icon } from "./icons";

/** 一组发言的反馈：这一轮现在的反馈，与给反馈的办法（成功返回 true）。 */
export interface AnswerFeedback {
  turnId: string;
  current: Feedback | null;
  give: (rating: Rating | null, extra?: { category?: FeedbackCategory; note?: string }) => Promise<boolean>;
}

/** 赞、踩两个键（动作行里）：已给的那个按下、实心。 */
export function FeedbackButtons({ feedback, open, onOpen }: { feedback: AnswerFeedback; open: Rating | null; onOpen: (rating: Rating) => void }) {
  const strings = useStrings();
  const rated = feedback.current?.rating ?? null;
  const button = (rating: Rating) => {
    const on = rated === rating;
    const label = rating === "up" ? strings.feedbackUp : strings.feedbackDown;
    const I = rating === "up" ? Icon.thumbUp : Icon.thumbDown;
    return (
      <button
        type="button"
        data-testid={`conv-feedback-${rating}`}
        data-state={on ? "on" : "off"}
        aria-pressed={on}
        aria-expanded={open === rating}
        aria-haspopup="dialog"
        title={label}
        aria-label={label}
        onClick={() => onOpen(rating)}
      >
        <I filled={on} />
      </button>
    );
  };
  return (
    <>
      {button("up")}
      {button("down")}
    </>
  );
}

/**
 * 给一个评分的对话框：踩时先问哪里不好（可不选），都可以补一句；已有反馈时带上它，能改、能撤。
 */
export function FeedbackDialog({
  feedback,
  rating,
  onClose,
  within,
}: {
  feedback: AnswerFeedback;
  rating: Rating;
  onClose: () => void;
  /** 打开它的那一行：点在那里不算点在外面。 */
  within?: RefObject<HTMLElement | null>;
}) {
  const strings = useStrings();
  const box = useRef<HTMLDivElement>(null);
  const same = feedback.current?.rating === rating ? feedback.current : null;
  const [category, setCategory] = useState<FeedbackCategory | null>(same?.category ?? null);
  const [note, setNote] = useState(same?.note ?? "");
  const [busy, setBusy] = useState(false);
  const layer = useLayer(box, { modal: false, close: onClose });
  useLayoutEffect(() => {
    const away = (event: MouseEvent) => {
      const target = event.target as Node;
      if (box.current?.contains(target) || within?.current?.contains(target)) return;
      layer.quiet();
      onClose();
    };
    window.addEventListener("mousedown", away, true);
    return () => window.removeEventListener("mousedown", away, true);
  }, [onClose]);

  const run = async (next: Rating | null) => {
    if (busy) return;
    setBusy(true);
    const ok = await feedback.give(next, next ? { ...(category ? { category } : {}), ...(note.trim() ? { note } : {}) } : {});
    setBusy(false);
    if (ok) onClose();
  };
  const keys = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      void run(rating);
      return;
    }
    layer.onKeyDown(event);
  };
  const title = rating === "up" ? strings.feedbackUpTitle : strings.feedbackDownTitle;
  return (
    <div
      ref={box}
      className="hx-fb"
      role="dialog"
      aria-label={title}
      aria-busy={busy || undefined}
      data-testid="conv-feedback-dialog"
      data-rating={rating}
      data-state={same ? "rated" : "new"}
      onKeyDown={keys}
    >
      <div className="hx-fb-title">{title}</div>
      {rating === "down" ? (
        <div className="hx-fb-cats" role="radiogroup" aria-label={strings.feedbackCategoryLabel}>
          {FEEDBACK_CATEGORIES.map((c) => (
            <button
              key={c}
              type="button"
              role="radio"
              aria-checked={category === c}
              className={`hx-fb-cat${category === c ? " on" : ""}`}
              data-testid="conv-feedback-category"
              data-id={c}
              onClick={() => setCategory((was) => (was === c ? null : c))}
            >
              {strings.feedbackCategory(c)}
            </button>
          ))}
        </div>
      ) : null}
      <textarea
        className="hx-fb-note"
        data-testid="conv-feedback-note"
        rows={3}
        maxLength={FEEDBACK_NOTE_MAX_CHARS}
        placeholder={strings.feedbackNote}
        aria-label={strings.feedbackNote}
        value={note}
        onChange={(e) => setNote(e.target.value)}
      />
      <div className="hx-fb-foot">
        <span className="hx-faint hx-fb-privacy">{strings.feedbackPrivacy}</span>
        {feedback.current ? (
          <button type="button" className="hx-btn hx-btn-ghost hx-btn-xs" data-testid="conv-feedback-retract" disabled={busy} onClick={() => void run(null)}>
            {strings.feedbackRetract}
          </button>
        ) : null}
        <button type="button" className="hx-btn hx-btn-ghost hx-btn-xs" data-testid="conv-feedback-cancel" onClick={onClose}>
          {strings.feedbackCancel}
        </button>
        <button type="button" className="hx-btn hx-btn-primary hx-btn-xs" data-testid="conv-feedback-submit" disabled={busy} onClick={() => void run(rating)}>
          {same ? strings.feedbackUpdate : strings.feedbackSubmit}
        </button>
      </div>
    </div>
  );
}
