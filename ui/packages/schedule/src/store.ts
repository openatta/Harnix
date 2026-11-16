/**
 * The schedules as the topic `schedule.list` last said them — every schedule of every space, and the
 * jobs models made in the engines (harnix-app.md §9) — followed while something on the page is
 * drawn from them: the rows' clocks, a project's block, the console, a session's list. And what each
 * of those asks of the list: the schedules of a session, of a project, the one that started a run.
 */

import type { ModelJob, Schedule, ScheduleList, Target } from "@harnix/core";
import type { HostApi } from "@harnix/react";

export const TOPIC = "schedule.list";

/** A schedule's state: on (due again), paused (switched off), ended (on, and due no more). */
export type ScheduleState = "on" | "paused" | "ended";

export function stateOf(s: Pick<Schedule, "enabled" | "next_fire">): ScheduleState {
  if (!s.enabled) return "paused";
  return s.next_fire == null ? "ended" : "on";
}

/** The session a target turns to: a session's, or the one a new session target opened and reuses. */
export function sessionOfTarget(target: Target): { instance: string; session_id: string } | null {
  if (target.kind === "session") return { instance: target.instance, session_id: target.session_id };
  if (target.kind === "new_session" && target.session) return target.session;
  return null;
}

/** Two directories, the same one: a trailing slash makes no difference. */
export function sameDir(a: string | null | undefined, b: string | null | undefined): boolean {
  if (!a || !b) return false;
  const norm = (p: string) => (p.length > 1 ? p.replace(/\/+$/, "") : p);
  return norm(a) === norm(b);
}

const EMPTY: ScheduleList = { schedules: [], model: [], unreachable: [] };

/** The topic's value, read without trusting its shape. */
export function listOf(value: unknown): ScheduleList {
  if (!value || typeof value !== "object") return EMPTY;
  const v = value as Partial<ScheduleList>;
  return { schedules: Array.isArray(v.schedules) ? v.schedules : [], model: Array.isArray(v.model) ? v.model : [], unreachable: Array.isArray(v.unreachable) ? v.unreachable : [] };
}

/** A session's schedules — those that turn to it, on or not — and the models' jobs in it. */
export function ofSession(list: ScheduleList, instance: string, sessionId: string): { schedules: Schedule[]; model: ModelJob[] } {
  return {
    schedules: list.schedules.filter((s) => {
      const to = sessionOfTarget(s.target);
      return to !== null && to.session_id === sessionId && to.instance === instance;
    }),
    model: list.model.filter((j) => j.session_id === sessionId && j.instance === instance),
  };
}

/** What makes a session's row show a clock: a schedule of it that is on, or a model's job in it. */
export function activeOfSession(list: ScheduleList, instance: string, sessionId: string): { schedules: Schedule[]; model: ModelJob[] } {
  const all = ofSession(list, instance, sessionId);
  return { schedules: all.schedules.filter((s) => stateOf(s) === "on"), model: all.model };
}

/** A project's schedules: those whose target works in it. */
export function ofProject(list: ScheduleList, root: string): Schedule[] {
  return list.schedules.filter((s) => sameDir(s.project, root));
}

/** The schedule a run says started it, while it is still there. */
export function scheduleOfRun(list: ScheduleList, run: { trigger?: { kind: string; schedule?: string } | null }): Schedule | null {
  if (run.trigger?.kind !== "schedule" || !run.trigger.schedule) return null;
  return list.schedules.find((s) => s.id === run.trigger!.schedule) ?? null;
}

/** Soonest due first, those due no more after them, then by title: as `schedule.list` orders them. */
export function ordered(schedules: Schedule[]): Schedule[] {
  return [...schedules].sort((a, b) => (a.next_fire ?? Infinity) - (b.next_fire ?? Infinity) || a.title.localeCompare(b.title));
}

/** The topic followed while someone listens; its last value at hand for every draw. */
export class ScheduleStore {
  private value: ScheduleList | undefined;
  private readonly listeners = new Set<() => void>();
  private off: (() => void) | null = null;

  constructor(private readonly host: HostApi) {}

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    if (!this.off) {
      try {
        this.off = this.host.subscribe(TOPIC, {}, (value) => {
          this.value = listOf(value);
          for (const listener of [...this.listeners]) listener();
        });
      } catch {
        // A page that may not follow the topic draws nothing from it.
        this.off = () => undefined;
      }
    }
    return () => {
      this.listeners.delete(fn);
      if (!this.listeners.size) {
        this.off?.();
        this.off = null;
        this.value = undefined;
      }
    };
  };

  /** The list as last said; undefined before the topic has said it. */
  readonly getSnapshot = (): ScheduleList | undefined => this.value;

  /** Stop following, whoever listens: the package is withdrawn. */
  stop(): void {
    this.off?.();
    this.off = null;
    this.listeners.clear();
    this.value = undefined;
  }
}
