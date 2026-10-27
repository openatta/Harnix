/**
 * 输入框：原型 composer(o) 的 DOM 与类名（加 hx- 前缀）。
 *
 *   .cmp
 *     提醒条（notice）、警示条（guard）           原型 .cmp-note / .cmp-note.warn / .cmp-note.bad
 *     .cmp-sugg   快捷回复（胶囊，最多 4 个）
 *     .cmp-box    [.pop 弹层] · 自动增高的 .cmp-ta（"/" 弹命令、"@" 弹条目）
 *       .cmp-bar  ＋ 添加 · ⚙ 选项 · [上下文环] · ── · [应用的控件] · 模型 · 🎤 · 发送 / 停止
 *     .cmp-hint   一行小字
 *
 * 这是所有输入的标准件：会话里的、新会话首页的、应用自己的，都用它。没有会话时 `state` 给 null。
 *
 * 发送只有一条路：打字发送、快捷回复、选中的命令都经 `send`（应用给的，或 controller.send）。
 * 运行中且没有输入时发送键变成停止（工位里是"中断当前步骤"，Esc 同效）。
 * ＋ 里是这条消息能带的东西：文件、图片（给了 `upload`）、截图、`@` 引用、`/` 命令。
 * 模型：给了 `onModel` 就能换（列表来自 `models`，也能输入名字），否则只显示当前的。
 * 后端还不支持的（截图、语音）照原型画出来，但不可用，弹层里写明原因。
 */

import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { useStrings } from "../context";
import type { SessionState } from "@harnix/core/state";
import { readDraft, writeDraft } from "./drafts";
import { compactTokens } from "./format";
import { Icon } from "./icons";
import type { ChatCommand, MentionItem, QuickReply } from "./types";

const QUICK_MAX = 4;

interface MenuItem {
  id: string;
  label: string;
  detail?: string;
  replacement: string;
}

interface Trigger {
  kind: "/" | "@";
  query: string;
  from: number;
  to: number;
}

/** 光标所在的触发：开头的 `/命令`，或任意位置、词首的 `@引用`（与旧界面 composer.js 相同的规则）。 */
function currentTrigger(value: string, caret: number): Trigger | null {
  if (value.startsWith("/") && !value.slice(0, caret).includes(" ") && !value.slice(0, caret).includes("\n")) {
    return { kind: "/", query: value.slice(1, caret), from: 0, to: caret };
  }
  const before = value.slice(0, caret);
  const at = before.lastIndexOf("@");
  if (at === -1) return null;
  const boundary = at === 0 || /\s/.test(before[at - 1]!);
  const token = before.slice(at + 1);
  if (!boundary || /\s/.test(token)) return null;
  return { kind: "@", query: token, from: at, to: caret };
}

/**
 * 上下文环的配置（ChatFeatures.contextRing 的对象形式）。环的值取会话的 `context`（turn_complete 与 session.get
 * 带的，引擎按厂商计数）；会话还没有调过模型时才用这里的 `ratio`。
 */
export interface ContextRingConfig {
  /** 上下文窗口（token）；会话的 context 没带 window 时用它 */
  max?: number;
  /** 应用给的占用比例（0..1）：会话还没有 context 时画它 */
  ratio?: number;
  /** 弹层里的行（例如工位的"压缩""清空"） */
  menu?: ReactNode;
  /** 弹层里百分比下面的一行小字 */
  note?: ReactNode;
  /** 环的 data-testid；缺省 conv-context-ring */
  testId?: string;
}

type PopKind = "plus" | "tools" | "model" | "ctx";

/** 已经传上去、等着随下一条消息发出的一个附件。 */
export interface Attached {
  label: string;
  /** 图片的预览（data: URL；CSP 不放 blob:） */
  preview?: string;
  /** 随 harnix.send 发出的那一项 */
  attachment: unknown;
}

/** 模型菜单里的一项。 */
export interface ModelOption {
  /** 发给后端的模型名 */
  id: string;
  /** 显示的名字；缺省用 id */
  label?: string;
  /** 哪个 provider 的 */
  provider?: string;
  /** 名字下面的一行小字 */
  detail?: string;
}

/** 从外面够得着的输入框。 */
export interface ComposerApi {
  /** 在光标处插入文字（没有焦点时接在末尾），并把焦点给输入框。 */
  insert(text: string): void;
}

export interface ChatComposerProps {
  state: SessionState | null;
  /** 一轮在跑（含应用经服务端发起的） */
  running?: boolean;
  /** 别的客户端在跑这个会话（提示一行） */
  elsewhere?: boolean;
  /** 发送：true ⇒ 已发、清空；false ⇒ 没发、保留原文 */
  send: (text: string, attachments: unknown[]) => Promise<boolean>;
  /** 输入框的句柄：应用往里插字（`useNest().composer.insert`）。卸载时给 null */
  bind?: (api: ComposerApi | null) => void;
  /** 上传一个文件；给了才能加附件（＋ 里的"上传文件"、粘贴、拖放）。失败时自己报，然后 reject */
  upload?: (file: File) => Promise<Attached>;
  interrupt: () => void;
  sendMode: "stop" | "separate";
  /** 工位转录：停止键写"中断当前步骤" */
  transcript?: boolean;
  disabled: boolean;
  placeholder: string;
  quickReplies: readonly QuickReply[];
  notice?: ReactNode;
  guard?: ReactNode;
  hint?: ReactNode;
  commands: readonly ChatCommand[] | null;
  mention?: (query: string) => Promise<MentionItem[]> | MentionItem[];
  contextRing: ContextRingConfig | null;
  /** ⚙ 弹层的内容；null ⇒ 不画 ⚙ */
  options: ReactNode | null;
  attachments: boolean;
  voice: boolean;
  model: string | null;
  /** 模型菜单里可选的模型 */
  models?: readonly ModelOption[];
  /** 换模型；给了菜单才可选（列表里的，或输入的名字）。reject ⇒ 没换（调用方自己报错） */
  onModel?: (model: ModelOption) => Promise<void> | void;
  /** 底栏右侧、模型前面的额外控件 */
  tools?: ReactNode;
  /** 草稿的键：同一个键的输入框共用未发送的内容（卸载再挂上仍在，见 drafts.ts）；不给 ⇒ 不保留 */
  draftKey?: string;
}

export function ChatComposer(props: ChatComposerProps) {
  const strings = useStrings();
  const { state } = props;
  const [text, setText] = useState(() => readDraft(props.draftKey));
  const [sending, setSending] = useState(false);
  const [menu, setMenu] = useState<{ kind: "/" | "@"; trigger: Trigger; items: MenuItem[]; index: number } | null>(null);
  const [pop, setPop] = useState<PopKind | null>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const [attached, setAttached] = useState<Attached[]>([]);
  const [uploading, setUploading] = useState(0);
  const [dropping, setDropping] = useState(false);
  const [switching, setSwitching] = useState(false);
  const [customModel, setCustomModel] = useState("");
  const mentionRequest = useRef(0);
  const running = props.running ?? state?.turn != null;
  const elsewhere = props.elsewhere ?? (!running && state?.session?.turn_state === "running");

  // 草稿：每次改动记下
  const draftKey = props.draftKey;
  useEffect(() => writeDraft(draftKey, text), [draftKey, text]);

  // 点框外关掉弹层
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!menu && !pop) return;
    const onDown = (e: MouseEvent) => {
      if (box.current && !box.current.contains(e.target as Node)) {
        setMenu(null);
        setPop(null);
      }
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [menu, pop]);

  const autoGrow = (node: HTMLTextAreaElement | null) => {
    if (!node) return;
    node.style.height = "auto";
    node.style.height = `${Math.min(node.scrollHeight, 220)}px`;
  };
  // 挂上时按草稿撑开高度
  useEffect(() => autoGrow(input.current), []);

  const { bind } = props;
  useEffect(() => {
    if (!bind) return;
    bind({
      insert(value: string) {
        const node = input.current;
        setText((current) => {
          const at = node && document.activeElement === node ? (node.selectionStart ?? current.length) : current.length;
          const next = current.slice(0, at) + value + current.slice(at);
          requestAnimationFrame(() => {
            if (!node) return;
            node.focus();
            node.setSelectionRange(at + value.length, at + value.length);
            autoGrow(node);
          });
          return next;
        });
      },
    });
    return () => bind(null);
  }, [bind]);

  const clear = () => {
    setText("");
    setAttached([]);
    setMenu(null);
    if (input.current) input.current.style.height = "auto";
  };

  /** 唯一的发送路径：打字、快捷回复都经这里 */
  const submitText = (value: string, fromInput: boolean) => {
    const message = value.trim();
    if (!message || props.disabled || sending || uploading) return;
    setSending(true);
    props
      .send(message, fromInput ? attached.map((a) => a.attachment) : [])
      .then((sent) => {
        if (sent && fromInput) clear();
      })
      .catch(() => undefined)
      .finally(() => {
        setSending(false);
        if (fromInput) input.current?.focus();
      });
  };

  const refreshMenu = (value: string, caret: number) => {
    const trigger = currentTrigger(value, caret);
    if (!trigger) {
      setMenu(null);
      return;
    }
    setPop(null);
    if (trigger.kind === "/") {
      const list = props.commands;
      if (!list?.length) {
        setMenu(null);
        return;
      }
      const q = trigger.query.toLowerCase();
      const items = list
        .filter((c) => c.name.toLowerCase().startsWith(q))
        .slice(0, 40)
        .map((c) => ({ id: c.name, label: `/${c.name}`, detail: c.description, replacement: `/${c.name} ` }));
      setMenu({ kind: "/", trigger, items, index: 0 });
      return;
    }
    const search = props.mention;
    if (!search) {
      setMenu(null);
      return;
    }
    const mine = ++mentionRequest.current;
    void Promise.resolve(search(trigger.query)).then(
      (found) => {
        if (mine !== mentionRequest.current) return;
        const items = found.slice(0, 8).map((m) => ({ id: m.id, label: m.id, detail: m.label, replacement: m.insert ?? `${m.id} ` }));
        setMenu({ kind: "@", trigger, items, index: 0 });
      },
      () => setMenu(null),
    );
  };

  const choose = (item: MenuItem | undefined) => {
    if (!menu || !item) return;
    const { from, to } = menu.trigger;
    const next = text.slice(0, from) + item.replacement + text.slice(to);
    setText(next);
    setMenu(null);
    const caret = from + item.replacement.length;
    requestAnimationFrame(() => {
      const node = input.current;
      if (!node) return;
      node.focus();
      node.setSelectionRange(caret, caret);
      autoGrow(node);
    });
  };

  /** ＋ 弹层的"命令"：开头换成 `/`，弹出命令菜单 */
  const insertSlash = () => {
    setPop(null);
    const next = text.startsWith("/") ? text : `/${text}`;
    setText(next);
    requestAnimationFrame(() => {
      const node = input.current;
      if (!node) return;
      node.focus();
      node.setSelectionRange(1, 1);
      autoGrow(node);
    });
    refreshMenu(next, 1);
  };

  /** 选文件：`accept` 只管这一次（图片 / 任意文件） */
  const pickFiles = (accept: string) => {
    setPop(null);
    const node = fileInput.current;
    if (!node) return;
    node.accept = accept;
    node.click();
  };

  const chooseModel = (option: ModelOption) => {
    const change = props.onModel;
    if (!change || switching) return;
    if (option.id === props.model) {
      setPop(null);
      return;
    }
    setSwitching(true);
    Promise.resolve(change(option))
      .then(() => {
        setPop(null);
        setCustomModel("");
      })
      .catch(() => undefined)
      .finally(() => setSwitching(false));
  };

  /** ＋ 弹层的"引用条目"：在光标处插一个 @，弹出条目菜单（原型 atInsert） */
  const insertAt = () => {
    setPop(null);
    const node = input.current;
    const caret = node?.selectionStart ?? text.length;
    const before = text.slice(0, caret);
    const lead = before && !/\s$/.test(before) ? " @" : "@";
    const next = before + lead + text.slice(caret);
    setText(next);
    const at = caret + lead.length;
    requestAnimationFrame(() => {
      if (!node) return;
      node.focus();
      node.setSelectionRange(at, at);
      autoGrow(node);
    });
    refreshMenu(next, at);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (menu) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        const n = Math.max(menu.items.length, 1);
        setMenu({ ...menu, index: (menu.index + (event.key === "ArrowDown" ? 1 : n - 1)) % n });
        return;
      }
      if ((event.key === "Tab" || (event.key === "Enter" && !event.shiftKey)) && menu.items.length) {
        event.preventDefault();
        choose(menu.items[menu.index]);
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        setMenu(null);
        return;
      }
    }
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      submitText(text, true);
    } else if (event.key === "Escape" && pop) {
      event.preventDefault();
      setPop(null);
    } else if (event.key === "Escape" && running) {
      event.preventDefault();
      props.interrupt();
    }
  };

  const addFiles = (files: Iterable<File>) => {
    const upload = props.upload;
    if (!upload) return;
    for (const file of files) {
      setUploading((n) => n + 1);
      upload(file)
        .then((item) => setAttached((list) => [...list, item]))
        .catch(() => undefined)
        .finally(() => setUploading((n) => n - 1));
    }
  };

  const toggle = (kind: PopKind) => {
    setMenu(null);
    setPop((p) => (p === kind ? null : kind));
  };

  const quick = props.quickReplies.slice(0, QUICK_MAX);
  const canSend = Boolean(text.trim()) && !props.disabled && !sending && !uploading;
  const showStop = props.sendMode === "stop" && running && !text.trim();
  const ring = props.contextRing ? ringValue(props.contextRing, state) : null;

  return (
    <div className="hx-cmp" data-testid="conv-composer" data-state={running ? "running" : props.disabled ? "disabled" : "idle"}>
      {props.notice}
      {props.guard}
      {elsewhere ? (
        <div className="hx-cmp-note" data-testid="conv-status" data-state="elsewhere">
          <span className="hx-spark" aria-hidden />
          <span>{strings.runningElsewhere}</span>
        </div>
      ) : null}
      {quick.length ? (
        <div className="hx-cmp-sugg" data-testid="conv-quick">
          {quick.map((q) => (
            <button
              key={q.label}
              type="button"
              data-testid="conv-quick-reply"
              disabled={props.disabled || sending}
              title={q.text && q.text !== q.label ? q.text : undefined}
              onClick={() => submitText(q.text ?? q.label, false)}
            >
              {q.label}
            </button>
          ))}
        </div>
      ) : null}
      <div
        className={`hx-cmp-box${dropping ? " dropping" : ""}`}
        ref={box}
        onDragOver={(event) => {
          if (!props.upload || !event.dataTransfer?.types.includes("Files")) return;
          event.preventDefault();
          setDropping(true);
        }}
        onDragLeave={() => setDropping(false)}
        onDrop={(event) => {
          setDropping(false);
          if (!props.upload || !event.dataTransfer?.files.length) return;
          event.preventDefault();
          addFiles(event.dataTransfer.files);
        }}
      >
        {props.upload ? (
          <input
            ref={fileInput}
            type="file"
            multiple
            hidden
            data-testid="conv-file"
            onChange={(event) => {
              const files = [...(event.target.files ?? [])];
              event.target.value = "";
              addFiles(files);
            }}
          />
        ) : null}
        {attached.length || uploading ? (
          <div className="hx-cmp-atts" data-testid="conv-attachments">
            {attached.map((item, i) => (
              <span key={`${item.label}-${i}`} className={item.preview ? "hx-att thumb" : "hx-att"} data-testid="conv-attachment">
                {item.preview ? <img src={item.preview} alt={item.label} /> : <Icon.file />}
                <span className="hx-truncate">{item.label}</span>
                <button
                  type="button"
                  className="hx-att-x"
                  data-testid="conv-attachment-remove"
                  title={strings.removeAttachment}
                  aria-label={strings.removeAttachment}
                  onClick={() => setAttached((list) => list.filter((_, j) => j !== i))}
                >
                  ×
                </button>
              </span>
            ))}
            {uploading ? <span className="hx-att busy">{strings.uploading}</span> : null}
          </div>
        ) : null}
        {menu ? (
          <div className="hx-pop" role="listbox" data-testid="conv-menu" data-kind={menu.kind}>
            <div className="hx-pop-h">{menu.kind === "/" ? strings.commands : strings.mentions}</div>
            {menu.items.length ? (
              menu.items.map((item, i) => (
                <button
                  key={item.id}
                  type="button"
                  role="option"
                  aria-selected={i === menu.index}
                  className={`hx-pop-it${i === menu.index ? " sel" : ""}`}
                  data-testid="conv-menu-item"
                  data-id={item.id}
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => choose(item)}
                >
                  <span className="hx-pop-pi" />
                  <span className="hx-pop-pt">
                    <b className="hx-mono">{item.label}</b>
                    {item.detail ? <small>{item.detail}</small> : null}
                  </span>
                  <span />
                </button>
              ))
            ) : (
              <div className="hx-pop-f">{menu.kind === "/" ? strings.noCommands : strings.noMentions}</div>
            )}
          </div>
        ) : pop === "plus" ? (
          <div className="hx-pop" role="menu" data-testid="conv-attach-menu">
            <div className="hx-pop-h">{strings.plusMenu}</div>
            <PopRow
              icon={<Icon.file />}
              title={strings.uploadFile}
              detail={props.upload ? undefined : strings.needsBackend}
              disabled={!props.upload || props.disabled}
              testId="conv-attach-file"
              onClick={() => pickFiles("")}
            />
            <PopRow
              icon={<Icon.image />}
              title={strings.uploadImage}
              detail={props.upload ? undefined : strings.needsBackend}
              disabled={!props.upload || props.disabled}
              testId="conv-attach-image"
              onClick={() => pickFiles("image/*")}
            />
            <PopRow icon={<Icon.shot />} title={strings.screenshot} detail={strings.needsBackend} disabled />
            <div className="hx-pop-sep" />
            <PopRow
              icon={<Icon.at />}
              title={strings.mentionItem}
              detail={props.mention ? strings.mentionItemHint : strings.needsBackend}
              disabled={!props.mention || props.disabled}
              testId="conv-attach-mention"
              onClick={insertAt}
            />
            <PopRow
              icon={<Icon.slash />}
              title={strings.insertCommand}
              detail={props.commands?.length ? strings.insertCommandHint : strings.noCommands}
              disabled={!props.commands?.length || props.disabled}
              testId="conv-attach-command"
              onClick={insertSlash}
            />
            <div className="hx-pop-f">{props.upload ? strings.attachFootReady : strings.attachFoot}</div>
          </div>
        ) : pop === "tools" && props.options ? (
          <div className="hx-pop" role="menu" data-testid="conv-options-menu">
            {props.options}
          </div>
        ) : pop === "model" && props.model ? (
          <div className="hx-pop right hx-models" role="menu" data-testid="conv-model-menu" aria-busy={switching}>
            <div className="hx-pop-h">{strings.model}</div>
            {modelChoices(props.model, props.models).map((option) => (
              <PopRow
                key={`${option.provider ?? ""}/${option.id}`}
                title={option.label ?? option.id}
                detail={[option.provider, option.detail].filter(Boolean).join(" · ") || undefined}
                on={option.id === props.model}
                disabled={!props.onModel || switching}
                testId="conv-model-option"
                onClick={() => chooseModel(option)}
              />
            ))}
            {props.onModel ? (
              <form
                className="hx-pop-input"
                onSubmit={(event) => {
                  event.preventDefault();
                  const id = customModel.trim();
                  if (id) chooseModel({ id });
                }}
              >
                <input
                  data-testid="conv-model-custom"
                  value={customModel}
                  disabled={switching}
                  placeholder={strings.modelCustom}
                  aria-label={strings.modelCustom}
                  onChange={(event) => setCustomModel(event.target.value)}
                />
              </form>
            ) : null}
            <div className="hx-pop-f">{props.onModel ? strings.modelFootSwitch : strings.modelFoot}</div>
          </div>
        ) : pop === "ctx" && ring ? (
          <div className="hx-pop" role="menu" data-testid="conv-context-menu">
            <div className="hx-pop-h">{strings.context}</div>
            <div className="hx-ctxpop">
              <div className="hx-ctxpop-row">
                <b className="hx-num">{ring.tokens ? compactTokens(ring.tokens) : `${Math.round((ring.ratio ?? 0) * 100)}%`}</b>
                <span className="hx-muted">
                  {ring.max ? `/ ${compactTokens(ring.max)}` : ""}
                  {ring.ratio != null && ring.tokens ? ` · ${Math.round(ring.ratio * 100)}%` : ""}
                </span>
              </div>
              {ring.ratio != null ? (
                <div className="hx-ctxbar">
                  <span className="bar">
                    <i className={ring.ratio > 0.6 ? "hi" : ""} style={{ width: `${Math.min(100, ring.ratio * 100).toFixed(0)}%` }} />
                  </span>
                </div>
              ) : null}
              {ring.threshold != null || props.contextRing?.note ? (
                <div className="hx-faint" style={{ fontSize: 12 }} data-testid="conv-context-note">
                  {ring.threshold != null ? strings.autoCompactAt(Math.round(ring.threshold * 100)) : null}
                  {ring.threshold != null && props.contextRing?.note ? " · " : null}
                  {props.contextRing?.note}
                </div>
              ) : null}
            </div>
            {props.contextRing?.menu}
          </div>
        ) : null}
        <textarea
          ref={input}
          className="hx-cmp-ta"
          data-testid="conv-input"
          rows={1}
          spellCheck={false}
          value={text}
          disabled={props.disabled}
          readOnly={sending}
          aria-label={props.placeholder}
          placeholder={props.placeholder}
          onChange={(event) => {
            setText(event.target.value);
            autoGrow(event.target);
            refreshMenu(event.target.value, event.target.selectionStart ?? event.target.value.length);
          }}
          onKeyDown={onKeyDown}
          onPaste={(event) => {
            const files = [...(event.clipboardData?.files ?? [])];
            if (!files.length || !props.upload) return;
            event.preventDefault();
            addFiles(files);
          }}
        />
        <div className="hx-cmp-bar">
          {props.attachments ? (
            <button
              type="button"
              className={`hx-cmp-ic${pop === "plus" ? " on" : ""}`}
              data-testid="conv-attach"
              title={strings.attach}
              aria-label={strings.attach}
              aria-expanded={pop === "plus"}
              onClick={() => toggle("plus")}
            >
              <Icon.plus />
            </button>
          ) : null}
          {props.options ? (
            <button
              type="button"
              className={`hx-cmp-ic${pop === "tools" ? " on" : ""}`}
              data-testid="conv-options"
              title={strings.options}
              aria-label={strings.options}
              aria-expanded={pop === "tools"}
              onClick={() => toggle("tools")}
            >
              <Icon.sliders />
            </button>
          ) : null}
          {ring ? (
            <ContextRing
              value={ring}
              testId={props.contextRing?.testId}
              open={pop === "ctx"}
              onClick={() => toggle("ctx")}
            />
          ) : null}
          <span className="hx-spacer" />
          {props.tools}
          {props.model ? (
            <button
              type="button"
              className={`hx-cmp-model${pop === "model" ? " on" : ""}`}
              data-testid="conv-model"
              title={strings.model}
              aria-label={`${strings.model}：${props.model}`}
              aria-expanded={pop === "model"}
              onClick={() => toggle("model")}
            >
              <Icon.cube />
              <span>{modelChoices(props.model, props.models).find((m) => m.id === props.model)?.label ?? props.model}</span>
              <Icon.down />
            </button>
          ) : null}
          {props.voice ? (
            <button type="button" className="hx-cmp-ic" data-testid="conv-voice" disabled title={`${strings.voice}（${strings.needsBackend}）`} aria-label={strings.voice}>
              <Icon.mic />
            </button>
          ) : null}
          {props.sendMode === "separate" ? (
            <button type="button" className="hx-cmp-ic" data-testid="conv-interrupt" disabled={!running} title={strings.interrupt} aria-label={strings.interrupt} onClick={props.interrupt}>
              <Icon.stop />
            </button>
          ) : null}
          {showStop ? (
            <button
              type="button"
              className="hx-cmp-send stop"
              data-testid="conv-interrupt"
              title={props.transcript ? strings.interruptStep : strings.stopHint}
              aria-label={props.transcript ? strings.interruptStep : strings.stop}
              onClick={props.interrupt}
            >
              <Icon.stop />
            </button>
          ) : (
            <button
              type="button"
              className="hx-cmp-send"
              data-testid="conv-send"
              disabled={!canSend}
              title={strings.sendHint}
              aria-label={strings.send}
              onClick={() => submitText(text, true)}
            >
              <Icon.up />
            </button>
          )}
        </div>
      </div>
      {props.hint ? <div className="hx-cmp-hint">{props.hint}</div> : null}
    </div>
  );
}

/** 模型菜单的行：给的列表，当前模型不在里面就放在最前 */
function modelChoices(current: string, models: readonly ModelOption[] | undefined): readonly ModelOption[] {
  const list = models ?? [];
  return list.some((m) => m.id === current) ? list : [{ id: current }, ...list];
}

/** 弹层的一行（原型 popRow）：图标 · 标题 / 说明 · 勾 */
function PopRow({
  icon,
  title,
  detail,
  disabled,
  on,
  testId,
  onClick,
}: {
  icon?: ReactNode;
  title: ReactNode;
  detail?: ReactNode;
  disabled?: boolean;
  on?: boolean;
  testId?: string;
  onClick?: () => void;
}) {
  return (
    <button type="button" role="menuitem" className="hx-pop-it" data-testid={testId} disabled={disabled} onClick={onClick}>
      <span className="hx-pop-pi">{icon}</span>
      <span className="hx-pop-pt">
        <b>{title}</b>
        {detail ? <small>{detail}</small> : null}
      </span>
      {on ? (
        <span className="hx-pop-pi ok">
          <Icon.check16 />
        </span>
      ) : (
        <span />
      )}
    </button>
  );
}

interface RingValue {
  tokens: number;
  max?: number;
  ratio?: number;
  /** 自动压缩开始的位置（占窗口的比例） */
  threshold?: number;
}

/**
 * 上下文环的值：会话的 `context`——最后一次调用发出去的上下文（`tokens`）、窗口（`window`）与自动压缩的位置
 * （`compact_threshold`）。不是本轮用量的合计：一轮调几次模型，合计就是上下文的几倍。
 */
function ringValue(config: ContextRingConfig, state: SessionState | null): RingValue | null {
  const context = state?.context;
  if (context && Number.isFinite(context.tokens)) {
    const max = context.window || config.max;
    const tokens = Math.max(0, context.tokens);
    return {
      tokens,
      max,
      ratio: max ? tokens / max : undefined,
      threshold: max && context.compact_threshold ? context.compact_threshold / max : undefined,
    };
  }
  if (config.ratio != null) return { tokens: 0, max: config.max, ratio: Math.max(0, config.ratio) };
  return null;
}

/** 上下文环（原型 ctxBtn）：百分比；点开是上下文弹层 */
function ContextRing({ value, testId, open, onClick }: { value: RingValue; testId?: string; open: boolean; onClick: () => void }) {
  const strings = useStrings();
  const p = value.ratio != null ? Math.min(1, value.ratio) : 0;
  const C = 2 * Math.PI * 6;
  const label = value.ratio != null && !value.tokens ? `${strings.context} ${Math.round(p * 100)}%` : strings.contextUsage(value.tokens, value.max ?? null);
  return (
    <button
      type="button"
      className={`hx-cmp-ctx${p > 0.6 ? " hi" : ""}${open ? " on" : ""}`}
      data-testid={testId ?? "conv-context-ring"}
      data-value={value.ratio != null ? String(Math.round(value.ratio * 1000) / 1000) : undefined}
      data-tokens={value.tokens}
      data-max={value.max}
      data-threshold={value.threshold != null ? String(Math.round(value.threshold * 1000) / 1000) : undefined}
      title={label}
      aria-label={label}
      aria-expanded={open}
      onClick={onClick}
    >
      <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
        <circle cx="8" cy="8" r="6" fill="none" stroke="var(--hx-line)" strokeWidth="2.2" />
        {value.ratio != null ? (
          <circle
            cx="8"
            cy="8"
            r="6"
            fill="none"
            stroke="currentColor"
            strokeWidth="2.2"
            strokeDasharray={`${(C * p).toFixed(1)} ${C.toFixed(1)}`}
            transform="rotate(-90 8 8)"
            strokeLinecap="round"
          />
        ) : null}
      </svg>
      <span className="hx-num">{value.ratio != null ? `${Math.round(p * 100)}%` : compactTokens(value.tokens)}</span>
    </button>
  );
}
