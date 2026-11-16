/**
 * 权限之外的三种卡：模型的提问、计划审阅、MCP server 的询问。读的是 core 给的 `form`（`@harnix/core/state` 的
 * `asks.ts`），不认引擎的帧。
 *
 * 和权限卡一样：任一观看者都能答、先答者生效，别处答了卡片就画出答案；倒计时到零按超时收起。表单都是原生控件，
 * 键盘能走完：Tab 换项、方向键换选项、Enter 在文字框里提交。
 */

import { useId, useState, type FormEvent, type ReactNode } from "react";
import type { AskAnswer, AskForm, BlockOf, ElicitField, ElicitProblem, ElicitValues, SessionController } from "@harnix/core/state";
import { elicitationContent } from "@harnix/core/state";
import { useStrings } from "../context";
import { useChat } from "../chat/types";
import { markdown } from "../markdown";
import type { HarnixUiStrings } from "../strings";

type Ask = BlockOf<"ask">;
type FormOf<T extends AskForm["type"]> = Extract<AskForm, { type: T }>;

/** 卡片的外框：头一行、内容、没答时的按钮和倒计时。 */
export function AskFrame({
  block,
  kind,
  label,
  head,
  children,
  actions,
  left,
}: {
  block: Ask;
  kind: AskForm["type"];
  label: string;
  head: string;
  children?: ReactNode;
  actions?: ReactNode;
  left: number;
}) {
  const strings = useStrings();
  const state = block.answered ? (block.answered === "timeout" ? "expired" : "answered") : "pending";
  return (
    <div
      className={`hx-blk hx-ask${block.answered ? " hx-ask-done" : ""}`}
      data-testid="conv-ask"
      data-kind={kind}
      data-id={block.prompt.prompt_id}
      data-state={state}
      data-answer={block.answered}
      role="group"
      aria-label={label}
    >
      <div className="hx-ask-head">{head}</div>
      {children}
      {block.answered || !actions ? null : (
        <div className="hx-ask-actions">
          {actions}
          {/* 倒计时不是装饰：没有它，等待未答提问的会话看起来像卡住了，其实正走向一次静默的拒绝。 */}
          <span className="hx-countdown" data-testid="conv-ask-countdown" aria-live="off">
            {strings.permCountdown(Math.max(0, left))}
          </span>
        </div>
      )}
    </div>
  );
}

/** 答了之后头一行说什么；别处答的再注明。 */
function outcome(strings: HarnixUiStrings, block: Ask, said: Partial<Record<AskAnswer, string>>): string {
  const answered = block.answered!;
  const common: Partial<Record<AskAnswer, string>> = {
    timeout: strings.askTimeout,
    withdrawn: strings.permDecisionWithdrawn,
    answered: strings.permDecisionElsewhere,
  };
  const text = said[answered] ?? common[answered] ?? strings.permDecisionElsewhere;
  const heard = block.elsewhere && answered !== "answered" && answered !== "timeout" && answered !== "withdrawn";
  return heard ? `${text} ${strings.askElsewhere}` : text;
}

/* ── 模型的提问 ──────────────────────────────────────────────────────── */

/** 选一个选项，或用自己的话答（没有选项时只有后者）；也可以不答。 */
export function QuestionAsk({ block, controller, left }: { block: Ask; controller: SessionController; left: number }) {
  const strings = useStrings();
  const form = block.form as FormOf<"question">;
  const name = useId();
  const OTHER = "\u0000other";
  const [choice, setChoice] = useState<string>(form.options.length ? "" : OTHER);
  const [text, setText] = useState("");
  const answer = choice === OTHER ? text.trim() : choice;
  const submit = (e?: FormEvent) => {
    e?.preventDefault();
    if (answer) void controller.reply(block.key, { type: "answer", text: answer });
  };
  const head = form.header ?? strings.askQuestion;
  const done = block.answered
    ? outcome(strings, block, { replied: strings.askReplied(block.reply ?? ""), declined: strings.askDeclined })
    : head;
  return (
    <AskFrame
      block={block}
      kind="question"
      label={head}
      head={done}
      left={left}
      actions={
        <>
          <button type="submit" form={name} className="hx-btn hx-btn-primary" data-testid="conv-ask-submit" disabled={!answer}>
            {strings.askSubmit}
          </button>
          <button type="button" className="hx-btn" data-testid="conv-ask-decline" onClick={() => void controller.reply(block.key, { type: "decline" })}>
            {strings.askDecline}
          </button>
        </>
      }
    >
      <div className="hx-ask-msg hx-ask-question" data-testid="conv-ask-question">
        {form.question}
      </div>
      {block.answered ? null : (
        <form id={name} className="hx-ask-form" onSubmit={submit}>
          {form.options.length ? (
            <div role="radiogroup" className="hx-ask-options" aria-label={form.question}>
              {form.options.map((o) => (
                <label key={o.key} className="hx-ask-option" data-testid="conv-ask-option" data-id={o.key}>
                  <input type="radio" name={name} value={o.key} checked={choice === o.key} onChange={() => setChoice(o.key)} />
                  <span>{o.label}</span>
                </label>
              ))}
              <label className="hx-ask-option" data-testid="conv-ask-other">
                <input type="radio" name={name} value={OTHER} checked={choice === OTHER} onChange={() => setChoice(OTHER)} />
                <span>{strings.askOther}</span>
              </label>
            </div>
          ) : null}
          {choice === OTHER ? (
            <input
              type="text"
              className="hx-ask-input"
              data-testid="conv-ask-other-text"
              aria-label={strings.askOther}
              placeholder={strings.askOtherPlaceholder}
              value={text}
              autoFocus={form.options.length > 0}
              onChange={(e) => setText(e.target.value)}
            />
          ) : null}
        </form>
      )}
    </AskFrame>
  );
}

/* ── 计划审阅 ────────────────────────────────────────────────────────── */

/** 计划按 Markdown 画；批准就离开计划模式，要求修改就把意见交给模型、留在计划模式。 */
export function PlanAsk({ block, controller, left }: { block: Ask; controller: SessionController; left: number }) {
  const strings = useStrings();
  const { untrusted } = useChat();
  const form = block.form as FormOf<"plan">;
  const [changing, setChanging] = useState(false);
  const [feedback, setFeedback] = useState("");
  const send = (e?: FormEvent) => {
    e?.preventDefault();
    const reason = feedback.trim();
    if (reason) void controller.answer(block.key, { type: "deny", reason });
  };
  const head = block.answered
    ? outcome(strings, block, {
        allow: strings.planApproved,
        deny: block.reply ? strings.planChangesRequested(block.reply) : strings.planChangesRequestedBare,
        timeout: strings.permDecisionTimeout,
      })
    : strings.planHead;
  const feedbackId = useId();
  return (
    <AskFrame
      block={block}
      kind="plan"
      label={strings.planHead}
      head={head}
      left={left}
      actions={
        changing ? (
          <>
            <button type="submit" form={feedbackId} className="hx-btn hx-btn-primary" data-testid="conv-ask-plan-send" disabled={!feedback.trim()}>
              {strings.planSendFeedback}
            </button>
            <button type="button" className="hx-btn" data-testid="conv-ask-plan-approve" onClick={() => void controller.answer(block.key, { type: "permit" })}>
              {strings.planApprove}
            </button>
          </>
        ) : (
          <>
            <button type="button" className="hx-btn hx-btn-primary" data-testid="conv-ask-plan-approve" onClick={() => void controller.answer(block.key, { type: "permit" })}>
              {strings.planApprove}
            </button>
            <button type="button" className="hx-btn" data-testid="conv-ask-plan-changes" onClick={() => setChanging(true)}>
              {strings.planRequestChanges}
            </button>
          </>
        )
      }
    >
      {form.plan ? (
        <div className="hx-ask-plan hx-md" data-testid="conv-ask-plan" dangerouslySetInnerHTML={{ __html: markdown(form.plan, { untrusted }) }} />
      ) : (
        <div className="hx-ask-msg" data-testid="conv-ask-plan">
          {strings.planEmpty}
        </div>
      )}
      {form.allowed.length ? (
        <div className="hx-ask-msg" data-testid="conv-ask-plan-allowed">
          <div className="hx-label">{strings.planAllowed}</div>
          <ul className="hx-ask-list">
            {form.allowed.map((a, i) => (
              <li key={i}>
                {a.prompt} <span className="hx-ask-faint">({a.tool})</span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      {changing && !block.answered ? (
        <form id={feedbackId} className="hx-ask-form" onSubmit={send}>
          <textarea
            className="hx-ask-input"
            data-testid="conv-ask-plan-feedback"
            aria-label={strings.planRequestChanges}
            placeholder={strings.planFeedbackPlaceholder}
            rows={3}
            value={feedback}
            autoFocus
            onChange={(e) => setFeedback(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) send();
            }}
          />
        </form>
      ) : null}
    </AskFrame>
  );
}

/* ── MCP server 的询问 ───────────────────────────────────────────────── */

const PROBLEM: Record<ElicitProblem, keyof HarnixUiStrings> = {
  required: "elicitRequired",
  number: "elicitNumber",
  integer: "elicitInteger",
  range: "elicitRange",
  length: "elicitLength",
  unsupported: "elicitUnsupported",
};

/**
 * 表单按 server 的 schema 画原始类型的字段；网址的只显示完整网址与主机，人自己点开，回来按「已完成」。
 * server 写的东西都标明出处。填的内容只交给 server，不广播给别的设备。
 */
export function ElicitationAsk({ block, controller, left }: { block: Ask; controller: SessionController; left: number }) {
  const strings = useStrings();
  const form = block.form as FormOf<"elicitation">;
  const formId = useId();
  const [values, setValues] = useState<ElicitValues>(() =>
    Object.fromEntries(form.fields.map((f) => [f.name, f.kind === "boolean" ? f.default === true : f.default != null ? String(f.default) : ""])),
  );
  const [problems, setProblems] = useState<Partial<Record<string, ElicitProblem>>>({});
  const submit = (e?: FormEvent) => {
    e?.preventDefault();
    if (form.url) {
      void controller.reply(block.key, { type: "accept" });
      return;
    }
    const read = elicitationContent(form.fields, values);
    if ("problems" in read) {
      setProblems(read.problems);
      return;
    }
    setProblems({});
    void controller.reply(block.key, { type: "accept", content: read.content });
  };
  const head = block.answered
    ? outcome(strings, block, { accepted: strings.elicitAccepted, declined: strings.elicitDeclined, cancelled: strings.elicitCancelled })
    : strings.elicitHead(form.server);
  return (
    <AskFrame
      block={block}
      kind="elicitation"
      label={strings.elicitHead(form.server)}
      head={head}
      left={left}
      actions={
        <>
          <button type="submit" form={formId} className="hx-btn hx-btn-primary" data-testid="conv-ask-elicit-accept">
            {form.url ? strings.elicitUrlDone : strings.elicitAccept}
          </button>
          <button type="button" className="hx-btn" data-testid="conv-ask-elicit-decline" onClick={() => void controller.reply(block.key, { type: "decline" })}>
            {strings.elicitDecline}
          </button>
          <button type="button" className="hx-btn" data-testid="conv-ask-elicit-cancel" onClick={() => void controller.reply(block.key, { type: "cancel" })}>
            {strings.elicitCancel}
          </button>
        </>
      }
    >
      <div className="hx-ask-faint" data-testid="conv-ask-elicit-from">
        {strings.elicitFrom(form.server)}
      </div>
      {form.message ? <div className="hx-ask-msg hx-ask-question">{form.message}</div> : null}
      {form.limited ? <div className="hx-ask-faint">{strings.elicitLimited}</div> : null}
      {block.answered ? null : form.url ? (
        <form id={formId} className="hx-ask-form" onSubmit={submit} data-testid="conv-ask-elicit-url">
          <code className="hx-ask-url">{form.url.href}</code>
          <div className="hx-ask-faint">{strings.elicitUrlHost(form.url.host)}</div>
          {/* Never opened for the person: they follow it themselves, having seen where it goes. */}
          <a className="hx-btn" href={form.url.href} target="_blank" rel="noopener noreferrer" data-testid="conv-ask-elicit-open">
            {strings.elicitOpenUrl}
          </a>
        </form>
      ) : (
        <form id={formId} className="hx-ask-form hx-ask-fields" onSubmit={submit} noValidate>
          {form.fields.map((field) => (
            <ElicitInput
              key={field.name}
              field={field}
              value={values[field.name]}
              problem={problems[field.name] ? String(strings[PROBLEM[problems[field.name]!]]) : undefined}
              onChange={(v) => setValues((old) => ({ ...old, [field.name]: v }))}
            />
          ))}
        </form>
      )}
    </AskFrame>
  );
}

function ElicitInput({
  field,
  value,
  problem,
  onChange,
}: {
  field: ElicitField;
  value: string | boolean | undefined;
  problem?: string;
  onChange: (value: string | boolean) => void;
}) {
  const strings = useStrings();
  const id = useId();
  const described = `${id}-help`;
  const label = `${field.title}${field.required ? " *" : ""}`;
  const aria = { "aria-invalid": problem ? true : undefined, "aria-describedby": described, "aria-required": field.required || undefined } as const;
  let input: ReactNode;
  switch (field.kind) {
    case "boolean":
      return (
        <div className="hx-ask-field" data-testid="conv-ask-elicit-field" data-id={field.name}>
          <label className="hx-ask-option">
            <input type="checkbox" checked={value === true} onChange={(e) => onChange(e.target.checked)} {...aria} />
            <span>{label}</span>
          </label>
          <FieldHelp id={described} field={field} problem={problem} />
        </div>
      );
    case "choice":
      input = (
        <select id={id} className="hx-ask-input" value={String(value ?? "")} onChange={(e) => onChange(e.target.value)} {...aria}>
          <option value="">{strings.elicitChoose}</option>
          {field.choices!.map((c) => (
            <option key={c.key} value={c.key}>
              {c.label}
            </option>
          ))}
        </select>
      );
      break;
    case "unsupported":
      input = <div className="hx-ask-faint">{strings.elicitUnsupported}</div>;
      break;
    default:
      input = (
        <input
          id={id}
          type="text"
          inputMode={field.kind === "number" ? "decimal" : field.kind === "integer" ? "numeric" : undefined}
          className="hx-ask-input"
          value={String(value ?? "")}
          onChange={(e) => onChange(e.target.value)}
          {...aria}
        />
      );
  }
  return (
    <div className="hx-ask-field" data-testid="conv-ask-elicit-field" data-id={field.name}>
      <label htmlFor={id} className="hx-label">
        {label}
      </label>
      {input}
      <FieldHelp id={described} field={field} problem={problem} />
    </div>
  );
}

function FieldHelp({ id, field, problem }: { id: string; field: ElicitField; problem?: string }) {
  if (!field.description && !problem) return <span id={id} hidden />;
  return (
    <div id={id} className="hx-ask-faint">
      {field.description}
      {problem ? (
        <span className="hx-ask-problem" role="alert" data-testid="conv-ask-elicit-problem">
          {field.description ? " · " : ""}
          {problem}
        </span>
      ) : null}
    </div>
  );
}
