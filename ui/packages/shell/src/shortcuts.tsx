/**
 * The shortcut service: every `shortcut` command on the page, the key each has now — its default
 * on this platform, or the one the person picked, kept in this browser under
 * `harnix.shortcuts.<command id>` (a local field store's shape) — the command a key press is for,
 * and the settings section that lists them and records new keys.
 *
 * A press runs the command whose key it is where the focus is: the composer's or the right column's
 * own before a global one, then the earliest registered (`@harnix/react`'s `shortcuts.ts`). While a
 * dialog is open none run: what is under it is not the page's to act on. Escape is the layers'.
 */

import { useEffect, useState, useSyncExternalStore, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { errorText } from "@harnix/core";
import {
  chordId,
  chordOf,
  defaultKeys,
  keyLabel,
  keysProblem,
  localFieldStore,
  modalUp,
  parseKeys,
  pickShortcut,
  registerTurnShortcuts,
  scopesOverlap,
  shortcutConflicts,
  shortcutPlatform,
  explainError,
  t,
  useRegistry,
  useT,
  writeKeys,
  type Chord,
  type FieldStore,
  type Registry,
  type ShortcutBinding,
  type ShortcutCommand,
  type ShortcutPlatform,
  type ShortcutScope,
} from "@harnix/react";

import type { App } from "./app.ts";
import { openSettings } from "./settings.tsx";

/** Where the person's own keys are kept: one entry per command, its key as written (`Meta+J`). */
export const SHORTCUT_PREFIX = "harnix.shortcuts.";

/** The shell's own commands' owner, as the other built-ins'. */
const OWNER = "builtin";

type Entry = ShortcutCommand & { owner: string };

/** A command as the settings list it. */
export interface ShortcutRow {
  id: string;
  owner: string;
  title: string;
  scope: ShortcutScope;
  /** Its key on this platform until the person picks another. */
  defaultChord: Chord | null;
  /** The key it has now. */
  chord: Chord | null;
  /** The person picked its key. */
  custom: boolean;
  /** The other commands with the same key where this one is in force too. */
  conflicts: string[];
}

export class Shortcuts {
  readonly platform: ShortcutPlatform;
  private readonly storage: Storage | null;
  /** The person's own keys, by command id, as written. */
  private overrides = new Map<string, string>();
  private readonly listeners = new Set<() => void>();
  private version = 0;

  constructor(
    private readonly registry: Registry,
    options: { platform?: ShortcutPlatform; storage?: Storage | null } = {},
  ) {
    this.platform = options.platform ?? shortcutPlatform();
    this.storage = options.storage ?? null;
    this.reload();
    // A command registered or withdrawn changes the keys in force.
    registry.subscribe(() => this.changed());
  }

  /** Read the person's keys again (after a write, or another tab's). */
  reload(): void {
    const next = new Map<string, string>();
    try {
      const s = this.storage;
      for (let i = 0; s && i < s.length; i += 1) {
        const key = s.key(i);
        if (!key?.startsWith(SHORTCUT_PREFIX)) continue;
        const value = JSON.parse(s.getItem(key) ?? "null") as unknown;
        if (typeof value === "string") next.set(key.slice(SHORTCUT_PREFIX.length), value);
      }
    } catch {
      /* unreadable storage: the defaults */
    }
    this.overrides = next;
    this.changed();
  }

  /** Follow another tab's changes to the keys. Returns the removal. */
  follow(): () => void {
    const onStorage = (event: StorageEvent) => {
      if (event.key === null || event.key.startsWith(SHORTCUT_PREFIX)) this.reload();
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  readonly getVersion = (): number => this.version;

  /** Every command with the key it has now, in registration order. */
  bindings(): (ShortcutBinding & { command: Entry; defaultChord: Chord | null; custom: boolean })[] {
    return this.registry.at("shortcut").map((command) => {
      const defaultChord = defaultKeys(command.keys, this.platform);
      const own = this.overrides.get(command.id);
      const picked = own === undefined ? null : parseKeys(own, this.platform);
      return { id: command.id, scope: command.scope ?? "global", chord: picked ?? defaultChord, command, defaultChord, custom: picked !== null };
    });
  }

  /** The commands as the settings list them. A title that throws is shown as the id. */
  rows(): ShortcutRow[] {
    const bindings = this.bindings();
    const conflicts = shortcutConflicts(bindings);
    return bindings.map((b) => {
      let title = b.id;
      try {
        title = b.command.title();
      } catch (e) {
        queueMicrotask(() => this.registry.fail("shortcut", b.command, e));
      }
      return { id: b.id, owner: b.command.owner, title, scope: b.scope, defaultChord: b.defaultChord, chord: b.chord, custom: b.custom, conflicts: conflicts.get(b.id) ?? [] };
    });
  }

  /** A command's key as a person reads it (`⌘K`); null while it has none. */
  label(id: string): string | null {
    const chord = this.bindings().find((b) => b.id === id)?.chord;
    return chord ? keyLabel(chord, this.platform) : null;
  }

  /** The command a press is for where the focus is (`scopes`), among those enabled now. */
  find(chord: Chord, scopes: readonly ShortcutScope[]): Entry | null {
    const enabled = this.bindings().filter((b) => b.chord && this.enabled(b.command));
    return pickShortcut(enabled, chord, scopes)?.command ?? null;
  }

  /** The keys the page takes wherever the focus is: what a frame hands to the shell
   *  (`Environment.keys`), as `chordId`s. */
  globalKeys(): string[] {
    return this.bindings()
      .filter((b) => b.scope === "global" && b.chord && this.enabled(b.command))
      .map((b) => chordId(b.chord!));
  }

  /** Why `chord` cannot be command `id`'s key: a key the page leaves alone, or one another command
   *  has where this one is in force too. Null when it can. */
  problem(id: string, chord: Chord): { key: string; vars?: Record<string, unknown> } | null {
    const reserved = keysProblem(chord, this.platform);
    if (reserved) return { key: reserved };
    const rows = this.rows();
    const self = rows.find((r) => r.id === id);
    if (!self) return null;
    const others = rows.filter((r) => r.id !== id && r.chord && chordId(r.chord) === chordId(chord) && scopesOverlap(r.scope, self.scope));
    return others.length ? { key: "shortcuts.problem.conflict", vars: { commands: others.map((r) => r.title).join(", ") } } : null;
  }

  /**
   * The keys as a field store (`localFieldStore`): a text field per command, its default the
   * key it has on this platform, `null` giving the person's key up. What it writes is read back
   * at once.
   */
  store(): FieldStore {
    const fields = this.registry.at("shortcut").map((command) => {
      const chord = defaultKeys(command.keys, this.platform);
      return { key: command.id, type: "text" as const, label: command.id, default: chord ? writeKeys(chord) : undefined };
    });
    const inner = localFieldStore(fields, { prefix: SHORTCUT_PREFIX, storage: this.storage ?? undefined });
    return {
      load: () => inner.load(),
      save: async (changes, revision) => {
        try {
          return await inner.save(changes, revision);
        } finally {
          this.reload();
        }
      },
    };
  }

  private enabled(command: Entry): boolean {
    try {
      return command.enabled?.() ?? true;
    } catch (e) {
      queueMicrotask(() => this.registry.fail("shortcut", command, e));
      return false;
    }
  }

  private changed(): void {
    this.version += 1;
    for (const fn of [...this.listeners]) fn();
  }
}

/* ── the shell's own commands ─────────────────────────────────────────── */

/** The shell's commands: a new conversation, the left column, the composer, the previous and next turn;
 *  the right column and the settings have no key until the person gives them one. */
export function registerShellShortcuts(app: App): void {
  const r = app.registry;
  r.register(
    "shortcut",
    {
      id: "shell.session.new",
      title: () => t("shortcuts.cmd.newSession"),
      keys: "Mod+K",
      // `[shell] new_session = false` (ipdrun-46): the key is not the shell's — left to the page.
      enabled: () => app.startsSessions(),
      run: () => document.querySelector<HTMLButtonElement>("#new")?.click(),
    },
    OWNER,
  );
  r.register(
    "shortcut",
    { id: "shell.sidebar.toggle", title: () => t("shortcuts.cmd.toggleSidebar"), keys: "Mod+\\", run: () => app.toggleSidebar() },
    OWNER,
  );
  r.register(
    "shortcut",
    {
      id: "shell.composer.focus",
      title: () => t("shortcuts.cmd.focusComposer"),
      keys: "Mod+/",
      run: () => document.querySelector<HTMLTextAreaElement>('[data-testid="conv-input"]')?.focus(),
    },
    OWNER,
  );
  r.register("shortcut", { id: "shell.details.toggle", title: () => t("shortcuts.cmd.toggleDetails"), run: () => app.toggleDetails() }, OWNER);
  r.register("shortcut", { id: "shell.settings.open", title: () => t("shortcuts.cmd.openSettings"), run: () => openSettings(app) }, OWNER);
  // ⌥↑ / ⌥↓: the turn before or after the one being read, in the conversation's turn rail.
  registerTurnShortcuts(r, OWNER);
}

/* ── dispatch ─────────────────────────────────────────────────────────── */

/** The places the focus is in, besides everywhere. */
export function scopesAt(target: EventTarget | null): ShortcutScope[] {
  const el = target instanceof Element ? target : null;
  const scopes: ShortcutScope[] = [];
  if (el?.closest('[data-testid="conv-composer"]')) scopes.push("composer");
  if (el?.closest("#details")) scopes.push("sidebar");
  scopes.push("global");
  return scopes;
}

function editable(target: EventTarget | null): boolean {
  const el = target instanceof HTMLElement ? target : null;
  return !!el && (el.isContentEditable || el instanceof HTMLTextAreaElement || el instanceof HTMLInputElement || el instanceof HTMLSelectElement);
}

/** The page's key listener for the commands. Returns its removal. */
export function listenForShortcuts(app: App): () => void {
  const onKey = (event: KeyboardEvent) => {
    if (modalUp() || event.defaultPrevented || event.isComposing) return;
    const chord = chordOf(event);
    if (!chord) return;
    // In a text field a key without Ctrl, ⌘ or ⌥ is typing.
    if (editable(event.target) && !chord.ctrl && !chord.meta && !chord.alt && !/^F\d+$/.test(chord.code)) return;
    const scopes = scopesAt(event.target);
    const command = app.shortcuts.find(chord, scopes);
    if (!command) return;
    event.preventDefault();
    const current = app.snapshot.current;
    const ctx = { space: app.snapshot.space, session: current ? { instance: current.instance, sessionId: current.sessionId } : null, scope: command.scope ?? "global" };
    try {
      void Promise.resolve(command.run(ctx)).catch((e: unknown) => app.notice(explainError(e), "error"));
    } catch (e) {
      app.notice(explainError(e), "error");
    }
  };
  window.addEventListener("keydown", onKey);
  const off = app.shortcuts.follow();
  return () => {
    window.removeEventListener("keydown", onKey);
    off();
  };
}

/* ── the settings section ─────────────────────────────────────────────── */

/**
 * Every command, its owner and its key; a key is recorded by pressing it (Escape gives up), refused
 * when the page leaves it alone or another command has it, and given up for the default one row at a
 * time or all at once.
 */
export function ShortcutsSection({ app }: { app: App }) {
  const t = useT();
  useRegistry(app.registry);
  const version = useSyncExternalStore(app.shortcuts.subscribe, app.shortcuts.getVersion);
  const service = app.shortcuts;
  const rows = service.rows();
  const [recording, setRecording] = useState<string | null>(null);
  const [warning, setWarning] = useState<{ id: string; text: string } | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [revision, setRevision] = useState<string | undefined>(undefined);
  useEffect(() => {
    let alive = true;
    void service
      .store()
      .load()
      .then((doc) => alive && setRevision(doc.revision))
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [service, version]);

  const save = async (changes: Record<string, unknown>, id: string | null) => {
    try {
      await service.store().save(changes, revision);
      setWarning(null);
    } catch (e) {
      setWarning({ id: id ?? "", text: errorText(e) });
    }
  };

  const record = (row: ShortcutRow, event: ReactKeyboardEvent<HTMLButtonElement>) => {
    if (recording !== row.id) return;
    if (event.key === "Tab") {
      setRecording(null);
      return;
    }
    // Every other key is the recording's: none reaches the dialog or the page.
    event.preventDefault();
    event.stopPropagation();
    const plain = !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey;
    if (event.key === "Escape" && plain) {
      setRecording(null);
      return;
    }
    const chord = chordOf(event.nativeEvent);
    if (!chord) return;
    setRecording(null);
    const problem = service.problem(row.id, chord);
    if (problem) {
      setWarning({ id: row.id, text: t(problem.key, problem.vars) });
      return;
    }
    if (row.chord && chordId(row.chord) === chordId(chord)) {
      setWarning(null);
      return;
    }
    const backToDefault = row.defaultChord && chordId(row.defaultChord) === chordId(chord);
    void save({ [row.id]: backToDefault ? null : writeKeys(chord) }, row.id);
  };

  const custom = rows.filter((r) => r.custom);
  return (
    <div className="shortcuts" data-testid="shortcuts">
      <h3>{t("shortcuts.title")}</h3>
      <div className="hint">{t("shortcuts.hint")}</div>
      <div className="shortcuts-bar">
        <span className="hint" data-testid="shortcuts-custom">
          {custom.length ? t("shortcuts.customCount", { count: custom.length }) : t("shortcuts.allDefault")}
        </span>
        {confirming ? (
          <>
            <button type="button" className="btn sm outline" data-testid="shortcuts-reset-all-cancel" onClick={() => setConfirming(false)}>
              {t("common.cancel")}
            </button>
            <button
              type="button"
              className="btn sm primary"
              data-testid="shortcuts-reset-all-confirm"
              onClick={() => {
                setConfirming(false);
                void save(Object.fromEntries(custom.map((r) => [r.id, null])), null);
              }}
            >
              {t("shortcuts.resetAllConfirm", { count: custom.length })}
            </button>
          </>
        ) : (
          <button type="button" className="btn sm outline" data-testid="shortcuts-reset-all" disabled={!custom.length} onClick={() => setConfirming(true)}>
            {t("shortcuts.resetAll")}
          </button>
        )}
      </div>
      {warning && !warning.id ? (
        <div className="hint err" data-testid="shortcut-warning">
          {warning.text}
        </div>
      ) : null}
      <div className="shortcut-rows">
        {rows.map((row) => {
          const owner = row.owner === OWNER ? t("shortcuts.owner.shell") : row.owner;
          const notes = [owner, row.scope === "global" ? "" : t(`shortcuts.scope.${row.scope}`), row.custom && row.defaultChord ? t("shortcuts.defaultWas", { keys: keyLabel(row.defaultChord, service.platform) }) : ""].filter(Boolean);
          const conflict = row.conflicts.length ? t("shortcuts.conflictWith", { commands: row.conflicts.map((id) => rows.find((r) => r.id === id)?.title ?? id).join(", ") }) : "";
          const on = recording === row.id;
          return (
            <div key={row.id} className="srow-setting shortcut-row" data-testid="shortcut-row" data-command={row.id} data-state={row.conflicts.length ? "conflict" : row.custom ? "custom" : undefined}>
              <div className="text">
                <div className="label">{row.title}</div>
                <div className="hint">
                  {notes.join(" · ")}
                  {conflict ? (
                    <span className="err" data-testid="shortcut-conflict">
                      {" · "}
                      {conflict}
                    </span>
                  ) : null}
                </div>
                {warning?.id === row.id ? (
                  <div className="hint err" data-testid="shortcut-warning">
                    {warning.text}
                  </div>
                ) : null}
              </div>
              <div className="control">
                <button
                  type="button"
                  className="shortcut-keys"
                  data-testid="shortcut-keys"
                  data-state={on ? "recording" : row.chord ? undefined : "none"}
                  aria-label={t("shortcuts.record", { command: row.title })}
                  onClick={() => {
                    setWarning(null);
                    setRecording(on ? null : row.id);
                  }}
                  onKeyDown={(event) => record(row, event)}
                  onBlur={() => on && setRecording(null)}
                >
                  {on ? t("shortcuts.press") : row.chord ? keyLabel(row.chord, service.platform) : t("shortcuts.none")}
                </button>
                {row.custom ? (
                  <button type="button" className="btn sm outline" data-testid="shortcut-reset" onClick={() => void save({ [row.id]: null }, row.id)}>
                    {t("shortcuts.reset")}
                  </button>
                ) : null}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
