/**
 * Settings drawn from declarations: a field says its key, its type, its default, which tiers may
 * write it, its text keys and whether a value of it is a risk; a store reads and writes the values.
 * [`FieldForm`] draws the rows for a store's fields — the control each type has, where the value
 * comes from (inherited, set in this tier, withheld in an untrusted project), "restore default" —
 * and writes through the store, confirming a risky value first and, when the store says the
 * values changed since they were read, offering to keep the change and look again.
 *
 * A `json` field is shown and not written: where its value stands and which file to edit.
 *
 * Two stores so far: the engine's settings (the `settings` extension's `describe` and `write`,
 * tiered) and this browser's own preferences ([`localFieldStore`], untiered). A `settings.section`
 * may declare `fields` (and a `store`) instead of drawing itself.
 */

import { createContext, useContext, useEffect, useId, useRef, useState, type ReactNode } from "react";
import { CONFLICT, errorText } from "@harnix/core";

import { Glyph } from "./glyph";
import { useT, type Translate } from "./i18n/index";
import { SettingsRow, useSettingsVersion } from "./settings";

export type FieldType = "toggle" | "select" | "number" | "text" | "secret" | "list" | "rules" | "route" | "json";

/** A provider a `route` field may send to, and the models it names. */
export interface FieldRoute {
  provider: string;
  default_model?: string | null;
  models?: string[];
}

/** A column of a `rules` field: text that is not blank, or one of its options. */
export interface FieldColumn {
  key: string;
  type?: "text" | "select";
  options?: string[];
  required?: boolean;
  /** Its heading's text key; the key itself when none. */
  label?: string;
  /** A text cell's own shape. `tool`: a permission rule's tool, `Name` or `Name(content)`
   *  ([`toolPattern`]); edited as the tool and what is in its parentheses. */
  check?: "tool";
}

/** Values the page confirms before writing; none listed is every value. */
export interface FieldRisk {
  values?: unknown[];
  /** The text key of what the confirmation says. */
  message: string;
}

/** What a field is: the same shape whoever stores it. */
export interface FieldDecl {
  key: string;
  type: FieldType;
  /** Text keys. The label is `settings.field.<key>` when none is given. */
  label?: string;
  hint?: string;
  /** The section it is drawn in, when a store's fields are spread over several. */
  section?: string;
  /** What it is when nothing sets it, when that can be said. "Restore default" goes back to it. */
  default?: unknown;
  options?: string[];
  /** Text keys of the options' names; one without a name is shown as it is. */
  optionLabels?: Record<string, string>;
  min?: number;
  max?: number;
  columns?: FieldColumn[];
  /** What a `text` field offers to pick from; a value not among them is accepted all the same. */
  suggestions?: string[];
  /** The providers a `route` field offers. Its value is a provider's id (its default model), or
   *  `{provider, model}`. */
  routes?: FieldRoute[];
  risk?: FieldRisk;
  /** The tiers that may write it (a tiered store's). */
  writable?: string[];
  /** Every tier's value applies side by side (a list of rules): a tier edits its own, and the
   *  value in force is all of them together. */
  additive?: boolean;
}

/** A field with where it stands, as a store reads it. */
export interface FieldState extends FieldDecl {
  /** The value in force. */
  effective?: unknown;
  /** The highest tier that writes it; none when no tier does. */
  source?: string | null;
  /** The project tier wrote a value that is not in force (an untrusted project may only tighten). */
  withheld?: boolean;
  /** Each tier's own value, `null` where it writes none. Absent for an untiered store. */
  tiers?: Record<string, unknown>;
  /** The tiers whose value the field does not accept (written by hand), and why. */
  invalid?: Record<string, string>;
}

export interface FieldDoc {
  fields: FieldState[];
  /** What a write names, so a store can refuse one made over values it did not read. */
  revision?: string;
}

export interface FieldStore<D extends FieldDoc = FieldDoc> {
  load(): Promise<D>;
  /** Writes `changes` as one; `null` gives a value up. Rejects with `{code: CONFLICT}` when
   *  `revision` is not the stored one, and with `data.errors: [{key, message}]` for values refused. */
  save(changes: Record<string, unknown>, revision?: string): Promise<unknown>;
}

/** Why `value` is not one the field accepts, as a text key and its values; null when it is. */
export function checkField(field: FieldDecl, value: unknown): { key: string; vars?: Record<string, unknown> } | null {
  const text = (v: unknown) => typeof v === "string" && v.trim() !== "";
  switch (field.type) {
    case "toggle":
      return typeof value === "boolean" ? null : { key: "settings.invalid.toggle" };
    case "select":
      return typeof value === "string" && (field.options ?? []).includes(value) ? null : { key: "settings.invalid.select" };
    case "number": {
      const min = field.min ?? Number.MIN_SAFE_INTEGER;
      const max = field.max ?? Number.MAX_SAFE_INTEGER;
      const ok = typeof value === "number" && Number.isInteger(value) && value >= min && value <= max;
      return ok ? null : { key: "settings.invalid.number", vars: { min: field.min ?? "−∞", max: field.max ?? "∞" } };
    }
    case "text":
    case "secret":
      return text(value) ? null : { key: "settings.invalid.text" };
    case "list":
      return Array.isArray(value) && value.every(text) ? null : { key: "settings.invalid.list" };
    case "json":
      return { key: "settings.invalid.readonly" };
    case "rules": {
      if (!Array.isArray(value)) return { key: "settings.invalid.rules", vars: { row: 1 } };
      const columns = field.columns ?? [];
      for (const [i, row] of value.entries()) {
        const bad = { key: "settings.invalid.rules", vars: { row: i + 1 } };
        if (!row || typeof row !== "object" || Array.isArray(row)) return bad;
        const cells = row as Record<string, unknown>;
        if (Object.keys(cells).some((k) => !columns.some((c) => c.key === k))) return bad;
        for (const column of columns) {
          const cell = cells[column.key];
          if (cell === undefined || cell === null) {
            if (column.required) return bad;
            continue;
          }
          const fits = column.options ? typeof cell === "string" && column.options.includes(cell) : text(cell);
          if (!fits) return bad;
          const shape = column.check === "tool" ? toolPattern(cell as string) : null;
          if (shape) return { key: shape, vars: { row: i + 1 } };
        }
      }
      return null;
    }
    case "route": {
      if (text(value)) return null;
      const route = value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
      const ok =
        route !== null &&
        text(route.provider) &&
        (route.model === undefined || route.model === null || text(route.model)) &&
        Object.keys(route).every((k) => k === "provider" || k === "model");
      return ok ? null : { key: "settings.invalid.route" };
    }
  }
}

/** A route's provider and model; the model is null when it is the provider's default. */
export function routeOf(value: unknown): { provider: string | null; model: string | null } {
  if (typeof value === "string") return { provider: value, model: null };
  if (value && typeof value === "object" && !Array.isArray(value)) {
    const route = value as Record<string, unknown>;
    return { provider: typeof route.provider === "string" ? route.provider : null, model: typeof route.model === "string" ? route.model : null };
  }
  return { provider: null, model: null };
}

/**
 * Why `text` is not a tool the engine's permission rules can name, as a text key; null when it is.
 * `Name` or `Name(content)`, the name `__`-separated segments of letters, digits, `-`, `_`, `.`
 * (`mcp__github` covers that server's tools); no wildcard in a name, no empty parentheses.
 */
export function toolPattern(text: string): string | null {
  const whole = text.trim();
  const open = whole.indexOf("(");
  if (open >= 0 && !whole.endsWith(")")) return "settings.invalid.toolParen";
  if (open < 0 && whole.includes(")")) return "settings.invalid.toolParen";
  const name = (open >= 0 ? whole.slice(0, open) : whole).trim();
  if (name.includes("*")) return "settings.invalid.toolWildcard";
  if (!name || !name.split("__").every((s) => /^[A-Za-z0-9_.-]+$/.test(s))) return "settings.invalid.toolName";
  if (open >= 0 && !whole.slice(open + 1, -1).trim()) return "settings.invalid.toolEmpty";
  return null;
}

/** A tool cell as the tool and what is in its parentheses: `Bash(git push:*)` → `Bash`, `git push:*`. */
function splitTool(text: string): { tool: string; content: string } {
  const open = text.indexOf("(");
  if (open < 0 || !text.endsWith(")")) return { tool: text, content: "" };
  return { tool: text.slice(0, open), content: text.slice(open + 1, -1) };
}

/** Not trimmed: a space typed at the end of a command is the start of its next word. */
function joinTool(tool: string, content: string): string {
  return content ? `${tool}(${content})` : tool;
}

/** Whether writing `value` is one the page asks about first. */
export function isRisky(field: FieldDecl, value: unknown): boolean {
  if (!field.risk || value === null || value === undefined) return false;
  const values = field.risk.values ?? [];
  return !values.length || values.some((v) => same(v, value));
}

function same(a: unknown, b: unknown): boolean {
  return JSON.stringify(a ?? null) === JSON.stringify(b ?? null);
}

/** A preference this browser keeps: in `localStorage` under the store's prefix, or wherever its
 *  own `get` and `set` keep it (the theme the shell applies, the language). */
export interface LocalField extends FieldDecl {
  get?(): unknown;
  set?(value: unknown): void;
}

/**
 * This browser's preferences as a field store: untiered, one value per field, `null` for the default.
 * Its revision is what is stored, so a page that read the values before another tab changed one is
 * told so before it writes over it.
 */
export function localFieldStore(fields: LocalField[], options: { prefix: string; storage?: Storage }): FieldStore {
  const storage = (): Storage | null => {
    try {
      return options.storage ?? globalThis.localStorage ?? null;
    } catch {
      return null;
    }
  };
  const read = (field: LocalField): unknown => {
    if (field.get) return field.get();
    try {
      const raw = storage()?.getItem(options.prefix + field.key);
      return raw === null || raw === undefined ? undefined : JSON.parse(raw);
    } catch {
      return undefined;
    }
  };
  const doc = (): FieldDoc => {
    const values = fields.map((field) => [field.key, read(field)] as const);
    return {
      fields: fields.map(({ get: _get, set: _set, ...field }, i) => ({ ...field, effective: values[i]![1] ?? field.default })),
      revision: JSON.stringify(values),
    };
  };
  return {
    load: async () => doc(),
    save: async (changes, revision) => {
      if (revision !== undefined && revision !== doc().revision) {
        throw { code: CONFLICT, message: "the preferences changed since they were read" };
      }
      const errors: { key: string; message: string }[] = [];
      for (const [key, value] of Object.entries(changes)) {
        const field = fields.find((f) => f.key === key);
        if (!field) errors.push({ key, message: `\`${key}\` is not a preference here` });
        else if (value !== null && checkField(field, value)) errors.push({ key, message: `\`${key}\` does not accept that value` });
      }
      if (errors.length) throw { code: -32602, message: errors.map((e) => e.message).join("; "), data: { errors } };
      for (const [key, value] of Object.entries(changes)) {
        const field = fields.find((f) => f.key === key)!;
        if (field.set) {
          field.set(value ?? field.default);
          continue;
        }
        try {
          if (value === null) storage()?.removeItem(options.prefix + key);
          else storage()?.setItem(options.prefix + key, JSON.stringify(value));
        } catch {
          /* private mode: the choice does not survive a reload */
        }
      }
    },
  };
}

/* ── the form ─────────────────────────────────────────────────────────── */

export interface FieldFormProps<D extends FieldDoc> {
  store: FieldStore<D>;
  /** Only this section's fields. */
  section?: string;
  /** The tier writes go to, for a store whose fields have `tiers`. */
  tier?: string;
  /** A tier's name, for the notes that say where a value comes from. */
  tierName?: (tier: string) => string;
  /** Above the rows, once the values are read: a tier picker, a note. */
  header?: (doc: D) => ReactNode;
  /** The section's heading, when the form is the whole of it. */
  title?: string;
  /** After the store took a write; with the values as they were read before it. */
  onSaved?: (changes: Record<string, unknown>, before: D | undefined) => void;
  /** After it refused one, other than for a revision that moved on (which the form handles). */
  onFailed?: (error: unknown) => void;
  /** The models an endpoint serves, offered beside the configured ones by a field that offers
   *  model names (`suggestions`) or routes. */
  listModels?: ModelLister;
}

/**
 * The models a provider's endpoint serves (`null`: the default provider's). `fresh` asks the
 * endpoint again rather than what was asked before. A rejection leaves the configured names, and a
 * name typed in is accepted all the same.
 */
export type ModelLister = (provider: string | null, fresh?: boolean) => Promise<string[]>;

const Models = createContext<ModelLister | null>(null);

/** What a control offers of a provider's endpoint: `provider` undefined is none to ask. */
function useFetchedModels(provider: string | null | undefined): { names: string[]; failed: boolean; refresh: (() => void) | null } {
  const lister = useContext(Models);
  const [names, setNames] = useState<string[]>([]);
  const [failed, setFailed] = useState(false);
  const [asked, setAsked] = useState(0);
  useEffect(() => {
    if (!lister || provider === undefined) return;
    let alive = true;
    lister(provider, asked > 0).then(
      (fetched) => {
        if (!alive) return;
        setNames(fetched);
        setFailed(false);
      },
      () => {
        if (!alive) return;
        setNames([]);
        setFailed(true);
      },
    );
    return () => {
      alive = false;
    };
  }, [lister, provider, asked]);
  return { names, failed, refresh: lister && provider !== undefined ? () => setAsked((n) => n + 1) : null };
}

/** The names configured, then the ones the endpoint serves, each once. */
function joined(configured: readonly string[], fetched: readonly string[]): string[] {
  return [...new Set([...configured, ...fetched])];
}

/** Ask the endpoint again; says when it could not be asked. */
function RefreshModels({ testId, fetched }: { testId: string; fetched: ReturnType<typeof useFetchedModels> }) {
  const t = useT();
  if (!fetched.refresh) return null;
  const why = fetched.failed ? t("settings.modelsFetchFailed") : t("settings.modelsRefresh");
  return (
    <button type="button" className="btn sm ghost" data-testid={`${testId}-models`} data-state={fetched.failed ? "failed" : "ok"} title={why} aria-label={why} onClick={fetched.refresh}>
      <Glyph name="refresh" className="glyph" />
    </button>
  );
}

type Problem = { key: string; vars?: Record<string, unknown> } | { text: string };

/** The rows for a store's fields, written through it. */
export function FieldForm<D extends FieldDoc>({ store, section, tier, tierName, header, title, onSaved, onFailed, listModels }: FieldFormProps<D>) {
  const t = useT();
  const version = useSettingsVersion();
  // The store a caller made this render; read through a ref so a new one does not read again.
  const latest = useRef(store);
  latest.current = store;
  const [nonce, setNonce] = useState(0);
  const [state, setState] = useState<{ doc?: D; error?: string } | null>(null);
  const [problems, setProblems] = useState<Record<string, Problem>>({});
  const [confirming, setConfirming] = useState<{ key: string; value: unknown } | null>(null);
  /** A write the store refused as made over values it did not read. */
  const [conflict, setConflict] = useState<Record<string, unknown> | null>(null);
  /** Changes kept through a conflict, shown over the values read again until written or dropped. */
  const [drafts, setDrafts] = useState<Record<string, unknown> | null>(null);
  const [moved, setMoved] = useState<string[]>([]);

  useEffect(() => {
    let alive = true;
    latest.current.load().then(
      (doc) => alive && setState({ doc }),
      (e: unknown) => alive && setState({ error: errorText(e) }),
    );
    return () => {
      alive = false;
    };
  }, [version, nonce]);

  const doc = state?.doc;
  const save = (changes: Record<string, unknown>) =>
    latest.current.save(changes, doc?.revision).then(
      () => {
        setProblems((p) => Object.fromEntries(Object.entries(p).filter(([k]) => !(k in changes))));
        setDrafts(null);
        setMoved([]);
        setNonce((n) => n + 1);
        onSaved?.(changes, doc);
      },
      (e: unknown) => {
        const error = e as { code?: number; data?: { errors?: { key: string; message: string }[] } };
        if (error?.code === CONFLICT) {
          setConflict(changes);
          return;
        }
        // Each value refused is said at its row; anything else at the first one written.
        const named = error?.data?.errors ?? [];
        const first = Object.keys(changes)[0];
        if (named.length) setProblems((p) => ({ ...p, ...Object.fromEntries(named.map((n) => [n.key, { text: n.message }])) }));
        else if (first) setProblems((p) => ({ ...p, [first]: { text: errorText(e) } }));
        onFailed?.(e);
      },
    );

  const commit = (field: FieldState, value: unknown, confirmed = false) => {
    const wrong = value === null ? null : checkField(field, value);
    setProblems((p) => {
      const { [field.key]: _gone, ...rest } = p;
      return wrong ? { ...rest, [field.key]: wrong } : rest;
    });
    if (wrong) return;
    if (!confirmed && isRisky(field, value)) {
      setConfirming({ key: field.key, value });
      return;
    }
    setConfirming(null);
    if (drafts) setDrafts({ ...drafts, [field.key]: value });
    else void save({ [field.key]: value });
  };

  /** Keep the refused change, read the values again, and show it over them. */
  const keepAndReview = async () => {
    const kept = conflict ?? {};
    setConflict(null);
    try {
      const fresh = await latest.current.load();
      const before = new Map((doc?.fields ?? []).map((f) => [f.key, f]));
      setMoved((fresh.fields ?? []).filter((f) => !same(before.get(f.key)?.effective, f.effective) || !same(before.get(f.key)?.tiers, f.tiers)).map((f) => f.key));
      setState({ doc: fresh });
      setDrafts({ ...(drafts ?? {}), ...kept });
    } catch (e) {
      setState({ error: errorText(e) });
    }
  };
  const drop = () => {
    setConflict(null);
    setDrafts(null);
    setMoved([]);
    setNonce((n) => n + 1);
  };

  const body = !state ? (
    <div className="hint">{t("common.loading")}</div>
  ) : state.error !== undefined ? (
    <div className="hint err" data-testid="settings-failed">
      {state.error}
    </div>
  ) : (
    <>
      {header ? header(state.doc!) : null}
      {conflict ? (
        <div className="callout warn settings-conflict" data-testid="settings-conflict">
          <span>{t("settings.conflict")}</span>
          <span className="sp" />
          <button type="button" className="btn sm outline" data-testid="settings-conflict-keep" onClick={() => void keepAndReview()}>
            {t("settings.conflictKeep")}
          </button>
          <button type="button" className="btn sm ghost" data-testid="settings-conflict-drop" onClick={drop}>
            {t("settings.conflictDrop")}
          </button>
        </div>
      ) : null}
      {drafts && !conflict ? (
        <div className="callout settings-review" data-testid="settings-review">
          <span>{t("settings.review")}</span>
          <span className="sp" />
          <button type="button" className="btn sm primary" data-testid="settings-review-save" onClick={() => void save(drafts)}>
            {t("settings.reviewSave")}
          </button>
          <button type="button" className="btn sm ghost" data-testid="settings-conflict-drop" onClick={drop}>
            {t("settings.conflictDrop")}
          </button>
        </div>
      ) : null}
      {(state.doc!.fields ?? [])
        .filter((field) => section === undefined || field.section === section)
        .map((field) => (
          <FieldRow
            key={field.key}
            t={t}
            field={field}
            tier={tier}
            tierName={tierName}
            draft={drafts && field.key in drafts ? { value: drafts[field.key] } : null}
            moved={moved.includes(field.key)}
            problem={problems[field.key]}
            confirming={confirming?.key === field.key ? confirming : null}
            commit={(value, confirmed) => commit(field, value, confirmed)}
            cancel={() => setConfirming(null)}
          />
        ))}
    </>
  );
  return (
    <Models.Provider value={listModels ?? null}>
      {title !== undefined ? (
        <>
          <h3>{title}</h3>
          <div>{body}</div>
        </>
      ) : (
        body
      )}
    </Models.Provider>
  );
}

/** The tiers below `tier`, in the order a field's `writable` lists them (lowest first). */
function below(field: FieldState, tier: string): string[] {
  const order = field.writable?.length ? field.writable : Object.keys(field.tiers ?? {});
  const at = order.indexOf(tier);
  return at < 0 ? [] : order.slice(0, at);
}

function FieldRow({
  t,
  field,
  tier,
  tierName,
  draft,
  moved,
  problem,
  confirming,
  commit,
  cancel,
}: {
  t: Translate;
  field: FieldState;
  tier?: string;
  tierName?: (tier: string) => string;
  draft: { value: unknown } | null;
  moved: boolean;
  problem?: Problem;
  confirming: { value: unknown } | null;
  commit: (value: unknown, confirmed?: boolean) => void;
  cancel: () => void;
}) {
  const [ack, setAck] = useState(false);
  const tiered = field.tiers !== undefined && tier !== undefined;
  const own = tiered ? field.tiers![tier!] : undefined;
  const inherited = own === null || own === undefined;
  const writable = !tiered || !field.writable || field.writable.includes(tier!);
  // An additive field edits this tier's own rows; the others' apply beside them.
  const additive = tiered && field.additive === true;
  const value = draft ? draft.value : additive ? (own ?? []) : field.effective;
  const others = additive ? Math.max(0, (Array.isArray(field.effective) ? field.effective.length : 0) - (Array.isArray(own) ? own.length : 0)) : 0;
  const name = (id: string) => (tierName ? tierName(id) : id);

  const notes: string[] = [];
  if (field.hint) notes.push(t(field.hint));
  if (field.type === "route" && !field.routes?.length) notes.push(t("settings.routeNoProviders"));
  if (tiered) {
    if (additive) notes.push(t("settings.additive", { count: others }));
    else notes.push(inherited ? t("settings.inherited") : t("settings.setHere"));
    if (field.source) notes.push(t("settings.sourceTier", { tier: name(field.source) }));
    if (field.withheld) notes.push(t("settings.withheld"));
    if (!writable) notes.push(t("settings.notWritableHere", { tiers: (field.writable ?? []).map(name).join(", ") }));
  }
  if (draft) notes.push(t("settings.draft", { value: shown(t, field, field.effective) }));
  if (moved) notes.push(t("settings.changedElsewhere"));
  const errors: string[] = [];
  for (const [at, why] of Object.entries(field.invalid ?? {})) errors.push(t("settings.invalidHere", { tier: name(at), why }));
  if (problem) errors.push("text" in problem ? problem.text : t(problem.key, problem.vars));

  const restore = () => {
    // Back to the default: given up in this tier when nothing below writes it, else written here.
    const lower = tiered && below(field, tier!).some((at) => field.tiers?.[at] !== null && field.tiers?.[at] !== undefined);
    commit(lower ? field.default : null);
  };
  const atDefault = field.default === undefined || same(value, field.default);

  return (
    <>
      <SettingsRow
        setting={field.key}
        state={draft ? "draft" : undefined}
        label={t(field.label ?? `settings.field.${field.key}`)}
        hint={
          notes.length || errors.length ? (
            <>
              {notes.join(" · ")}
              {errors.map((e, i) => (
                <span key={i} className="err" data-testid="setting-problem">
                  {notes.length || i ? " · " : ""}
                  {e}
                </span>
              ))}
            </>
          ) : undefined
        }
      >
        {field.type === "list" || field.type === "rules" ? null : <Control field={field} value={value} disabled={!writable} commit={commit} />}
        {tiered && !inherited && writable ? (
          <button type="button" className="btn sm outline" data-testid={`settings-${field.key}-clear`} title={t("settings.clearHint")} onClick={() => commit(null)}>
            {t("settings.clear")}
          </button>
        ) : null}
        {!atDefault && writable ? (
          <button type="button" className="btn sm ghost" data-testid={`settings-${field.key}-default`} title={t("settings.restoreDefaultHint", { value: shown(t, field, field.default) })} onClick={restore}>
            {t("settings.restoreDefault")}
          </button>
        ) : null}
      </SettingsRow>
      {field.type === "list" || field.type === "rules" ? (
        <div className="srow-wide" data-setting={field.key}>
          <Control field={field} value={value} disabled={!writable} commit={commit} />
        </div>
      ) : null}
      {confirming ? (
        <div className="callout warn settings-risk" data-testid="settings-risk" data-setting={field.key}>
          <span>{t(field.risk!.message)}</span>
          <label className="check">
            <input type="checkbox" data-testid="settings-risk-ack" checked={ack} onChange={(e) => setAck(e.target.checked)} />
            {t("settings.riskAck")}
          </label>
          <span className="sp" />
          <button type="button" className="btn sm ghost" data-testid="settings-risk-cancel" onClick={cancel}>
            {t("common.cancel")}
          </button>
          <button type="button" className="btn sm outline danger" data-testid="settings-risk-confirm" disabled={!ack} onClick={() => commit(confirming.value, true)}>
            {t("common.confirm")}
          </button>
        </div>
      ) : null}
    </>
  );
}

/** A value as a note says it. */
function shown(t: Translate, field: FieldDecl, value: unknown): string {
  if (value === null || value === undefined) return t("common.none");
  if (field.type === "secret") return String(value);
  if (field.type === "toggle") return value ? t("settings.on") : t("settings.off");
  if (field.type === "select" && typeof value === "string") return optionName(t, field, value);
  if (Array.isArray(value)) return field.type === "rules" ? t("settings.rulesCount", { count: value.length }) : value.join(", ");
  if (field.type === "route") {
    const { provider, model } = routeOf(value);
    return model ? `${provider} · ${model}` : String(provider);
  }
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}

function optionName(t: Translate, field: FieldDecl, option: string): string {
  const key = field.optionLabels?.[option];
  return key ? t(key) : option;
}

function Control({ field, value, disabled, commit }: { field: FieldState; value: unknown; disabled: boolean; commit: (value: unknown) => void }) {
  const t = useT();
  const testId = `settings-${field.key}`;
  switch (field.type) {
    case "toggle":
      return <input type="checkbox" data-testid={testId} disabled={disabled} checked={Boolean(value)} onChange={(e) => commit(e.target.checked)} />;
    case "select": {
      const options = field.options ?? [];
      const current = typeof value === "string" ? value : "";
      // A value the options lack (written by hand), or none: shown as it is, not as the first option.
      const extra = options.includes(current) ? [] : [current];
      return (
        <select data-testid={testId} disabled={disabled} value={current} onChange={(e) => commit(e.target.value)}>
          {[...extra, ...options].map((option) => (
            <option key={option} value={option} disabled={!options.includes(option)}>
              {option === "" ? t("common.none") : optionName(t, field, option)}
            </option>
          ))}
        </select>
      );
    }
    case "number":
    case "text":
      return <TextControl key={String(value ?? "")} testId={testId} field={field} value={value} disabled={disabled} commit={commit} />;
    case "secret":
      return <SecretControl testId={testId} value={value} disabled={disabled} commit={commit} />;
    case "list":
      return <ListControl key={JSON.stringify(value ?? null)} testId={testId} value={value} disabled={disabled} commit={commit} />;
    case "json":
      return <code data-testid={testId}>{value === null || value === undefined ? t("common.none") : JSON.stringify(value)}</code>;
    case "rules":
      return <RulesControl key={JSON.stringify(value ?? null)} testId={testId} field={field} value={value} disabled={disabled} commit={commit} />;
    case "route":
      return <RouteControl key={JSON.stringify(value ?? null)} testId={testId} field={field} value={value} disabled={disabled} commit={commit} />;
  }
}

/** Text or a number, written when the input is left with something new in it. */
function TextControl({ testId, field, value, disabled, commit }: { testId: string; field: FieldState; value: unknown; disabled: boolean; commit: (value: unknown) => void }) {
  const start = value === null || value === undefined ? "" : String(value);
  const [text, setText] = useState(start);
  const list = useId();
  const number = field.type === "number";
  // A field that offers model names offers the default provider's endpoint's too.
  const fetched = useFetchedModels(!number && field.suggestions !== undefined ? null : undefined);
  const suggestions = number ? [] : joined(field.suggestions ?? [], fetched.names);
  return (
    <>
      <input
        type={number ? "number" : "text"}
        data-testid={testId}
        disabled={disabled}
        value={text}
        min={number ? field.min : undefined}
        max={number ? field.max : undefined}
        list={suggestions.length ? list : undefined}
        onChange={(e) => setText(e.target.value)}
        onBlur={() => {
          if (text === start || !text.trim()) return;
          commit(number ? Number(text) : text);
        }}
      />
      {suggestions.length ? <Suggestions id={list} names={suggestions} /> : null}
      {disabled ? null : <RefreshModels testId={testId} fetched={fetched} />}
    </>
  );
}

function Suggestions({ id, names }: { id: string; names: readonly string[] }) {
  return (
    <datalist id={id}>
      {names.map((name) => (
        <option key={name} value={name} />
      ))}
    </datalist>
  );
}

/**
 * A provider, picked from the ones configured, and a model of it: typed or picked from the ones it
 * names, blank for its default. Written as the provider's id alone when the model is its default.
 */
function RouteControl({ testId, field, value, disabled, commit }: { testId: string; field: FieldState; value: unknown; disabled: boolean; commit: (value: unknown) => void }) {
  const t = useT();
  const list = useId();
  const routes = field.routes ?? [];
  const { provider, model } = routeOf(value);
  const [text, setText] = useState(model ?? "");
  const known = routes.find((r) => r.provider === provider);
  const fetched = useFetchedModels(provider ?? undefined);
  const models = joined(known?.models ?? [], fetched.names);
  const ids = routes.map((r) => r.provider);
  // A provider the options lack (gone since, or written by hand), or none: shown as it is.
  const extra = provider === null ? [""] : ids.includes(provider) ? [] : [provider];
  const to = (provider: string, model: string) => (model ? { provider, model } : provider);
  return (
    <span className="settings-route">
      <select data-testid={testId} disabled={disabled || !routes.length} value={provider ?? ""} onChange={(e) => commit(e.target.value)}>
        {[...extra, ...ids].map((id) => (
          <option key={id} value={id} disabled={!ids.includes(id)}>
            {id === "" ? t("common.none") : id}
          </option>
        ))}
      </select>
      <input
        type="text"
        data-testid={`${testId}-model`}
        disabled={disabled || provider === null}
        value={text}
        placeholder={known?.default_model ?? t("settings.routeDefaultModel")}
        list={models.length ? list : undefined}
        onChange={(e) => setText(e.target.value)}
        onBlur={() => {
          const next = text.trim();
          if (provider === null || next === (model ?? "")) return;
          commit(to(provider, next));
        }}
      />
      {models.length ? <Suggestions id={list} names={models} /> : null}
      {disabled ? null : <RefreshModels testId={testId} fetched={fetched} />}
    </span>
  );
}

/** A secret is written, never shown: the field says only that one is set. */
function SecretControl({ testId, value, disabled, commit }: { testId: string; value: unknown; disabled: boolean; commit: (value: unknown) => void }) {
  const t = useT();
  const [text, setText] = useState("");
  const set = value !== null && value !== undefined && value !== "";
  return (
    <input
      type="password"
      autoComplete="off"
      data-testid={testId}
      disabled={disabled}
      value={text}
      placeholder={set ? t("settings.secretSet", { value: String(value) }) : t("settings.secretUnset")}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => {
        if (!text) return;
        commit(text);
        setText("");
      }}
    />
  );
}

/** One item a line. */
function ListControl({ testId, value, disabled, commit }: { testId: string; value: unknown; disabled: boolean; commit: (value: unknown) => void }) {
  const t = useT();
  const items = Array.isArray(value) ? value.map(String) : [];
  const [text, setText] = useState(items.join("\n"));
  return (
    <>
      <textarea
        data-testid={testId}
        disabled={disabled}
        rows={Math.min(8, Math.max(3, items.length + 1))}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onBlur={() => {
          const next = text.split("\n").map((s) => s.trim()).filter(Boolean);
          if (!same(next, items)) commit(next);
        }}
      />
      <div className="hint">{t("settings.listHint")}</div>
    </>
  );
}

/** Rows of the field's columns, edited here and written together. */
function RulesControl({ testId, field, value, disabled, commit }: { testId: string; field: FieldState; value: unknown; disabled: boolean; commit: (value: unknown) => void }) {
  const t = useT();
  const columns = field.columns ?? [];
  const start = Array.isArray(value) ? (value as Record<string, unknown>[]) : [];
  const [rows, setRows] = useState<Record<string, unknown>[]>(start);
  const change = (i: number, key: string, cell: string) =>
    setRows(rows.map((row, at) => (at === i ? cleaned({ ...row, [key]: cell }) : row)));
  const blank = (): Record<string, unknown> =>
    Object.fromEntries(columns.filter((c) => c.required && c.options?.length).map((c) => [c.key, c.options![0]]));
  return (
    <div className="settings-rules" data-testid={testId}>
      <table>
        <thead>
          <tr>
            {columns.map((c) => (
              <th key={c.key}>{c.label ? t(c.label) : c.key}</th>
            ))}
            <th />
          </tr>
        </thead>
        <tbody>
          {rows.map((row, i) => (
            <tr key={i} data-row={i}>
              {columns.map((c) => (
                <td key={c.key}>
                  {c.options ? (
                    <select disabled={disabled} value={String(row[c.key] ?? "")} onChange={(e) => change(i, c.key, e.target.value)}>
                      {c.required ? null : <option value="">{t("common.none")}</option>}
                      {c.options.map((o) => (
                        <option key={o} value={o}>
                          {o}
                        </option>
                      ))}
                    </select>
                  ) : c.check === "tool" ? (
                    <ToolCell disabled={disabled} value={String(row[c.key] ?? "")} onChange={(cell) => change(i, c.key, cell)} />
                  ) : (
                    <input type="text" disabled={disabled} value={String(row[c.key] ?? "")} onChange={(e) => change(i, c.key, e.target.value)} />
                  )}
                </td>
              ))}
              <td>
                <button type="button" className="btn sm ghost" disabled={disabled} data-testid="settings-rule-remove" onClick={() => setRows(rows.filter((_, at) => at !== i))}>
                  {t("common.remove")}
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {!rows.length ? <div className="hint">{t("settings.rulesEmpty")}</div> : null}
      <div className="settings-rules-actions">
        <button type="button" className="btn sm outline" disabled={disabled} data-testid="settings-rule-add" onClick={() => setRows([...rows, blank()])}>
          {t("common.add")}
        </button>
        {!same(rows, start) ? (
          <button type="button" className="btn sm primary" disabled={disabled} data-testid="settings-rules-apply" onClick={() => commit(rows.map((row) => trimmed(row, columns)))}>
            {t("common.save")}
          </button>
        ) : null}
      </div>
    </div>
  );
}

/** A permission rule's tool, edited as the tool's name and the path, command or URL pattern its
 *  parentheses hold — the same pair a flow's permission writes as `tools` and `paths`/`commands`. */
function ToolCell({ value, disabled, onChange }: { value: string; disabled: boolean; onChange: (cell: string) => void }) {
  const t = useT();
  const { tool, content } = splitTool(value);
  return (
    <span className="settings-rule-tool">
      <input type="text" data-testid="settings-rule-tool" disabled={disabled} placeholder="Bash" value={tool} onChange={(e) => onChange(joinTool(e.target.value, content))} />
      <input type="text" data-testid="settings-rule-content" disabled={disabled} placeholder={t("settings.ruleContent")} title={t("settings.ruleContentHint")} value={content} onChange={(e) => onChange(joinTool(tool, e.target.value))} />
    </span>
  );
}

/** A row as it is written: its text without the spaces around it, a tool's inside its parentheses too. */
function trimmed(row: Record<string, unknown>, columns: FieldColumn[]): Record<string, unknown> {
  const trim = (key: string, v: unknown): unknown => {
    if (typeof v !== "string") return v;
    if (columns.find((c) => c.key === key)?.check !== "tool") return v.trim();
    const { tool, content } = splitTool(v.trim());
    return joinTool(tool.trim(), content.trim());
  };
  return cleaned(Object.fromEntries(Object.entries(row).map(([k, v]) => [k, trim(k, v)])));
}

/** A row without its empty cells: an optional column left empty is not written. */
function cleaned(row: Record<string, unknown>): Record<string, unknown> {
  return Object.fromEntries(Object.entries(row).filter(([, v]) => v !== "" && v !== null && v !== undefined));
}

/** For a section that only declares fields: its form, on the store it names or this browser's. */
export function FieldsSection({ fields, store, owner }: { fields: LocalField[]; store?: FieldStore; owner: string }) {
  const [fallback] = useState(() => localFieldStore(fields, { prefix: `harnix.pref.${owner}.` }));
  return <FieldForm store={store ?? fallback} />;
}
