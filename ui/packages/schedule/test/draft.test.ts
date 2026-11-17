/**
 * The editor's draft: what it opens with (an edit's schedule, else what the opener knows), what is
 * wrong with it for each target, and what it is sent as — a create, or an update of what changed
 * alone against the revision it read (an `every` left as it was not resent).
 */

import { describe, expect, it } from "vitest";

import { changedFields, changesAnything, createParams, draftProblems, initialDraft, rebase, targetOf, updateParams, type Draft, type EditorContext } from "../src/draft";
import { inputFields } from "../src/inputs";
import { ROOT, schedule } from "./fixtures";

const NOW = Date.parse("2026-11-16T00:00:00Z");
const SESSION: EditorContext = { kinds: ["session"], space: "sessions", session: { instance: "main", session_id: "s1", name: "Chat" } };
const filled = (ctx: EditorContext, over: Partial<Draft> = {}): Draft => ({ ...initialDraft(ctx, NOW), zone: "Asia/Shanghai", ...over });

describe("a new schedule's draft", () => {
  it("opens daily at nine in this machine's zone, on the opener's target and project", () => {
    const d = initialDraft({ kinds: ["task", "new_session"], project: ROOT, task: { flow: "example-review", subject: "the repo", workspace: "worktree" } }, NOW);
    expect(d).toMatchObject({ kind: "task", project: ROOT, flow: "example-review", subject: "the repo", workspace: "worktree", missed: "catch_up_latest", concurrency: "skip", mode: "new_once_then_reuse" });
    expect(d.freq).toMatchObject({ kind: "daily", time: "09:00" });
    expect(d.zone).toBeTruthy();
  });

  it("a session target needs a title and what to say; a session to send to", () => {
    expect(draftProblems(filled(SESSION), SESSION, [], NOW)).toEqual({ title: "schedule.err.title", prompt: "schedule.err.prompt" });
    expect(draftProblems(filled(SESSION, { title: "x".repeat(121), prompt: "hi" }), SESSION, [], NOW)).toEqual({ title: "schedule.err.titleLong" });
    expect(draftProblems(filled(SESSION, { title: "t", prompt: "hi" }), { kinds: ["session"] }, [], NOW)).toEqual({ session: "schedule.err.session" });
    expect(draftProblems(filled(SESSION, { title: "t", prompt: "hi", zone: "Nowhere/Else" }), SESSION, [], NOW)).toEqual({ zone: "schedule.err.zone" });
    expect(draftProblems(filled(SESSION, { title: "t", prompt: "hi" }), SESSION, [], NOW)).toEqual({});
  });

  it("a new session's project, when given, is an absolute directory", () => {
    const ctx: EditorContext = { kinds: ["new_session"] };
    expect(draftProblems(filled(ctx, { title: "t", prompt: "hi" }), ctx, [], NOW)).toEqual({});
    expect(draftProblems(filled(ctx, { title: "t", prompt: "hi", project: "work/alpha" }), ctx, [], NOW)).toEqual({ project: "schedule.err.project" });
  });

  it("a task needs a flow, a subject, a project and its required inputs", () => {
    const ctx: EditorContext = { kinds: ["task"] };
    const fields = inputFields({ type: "object", properties: { target: { type: "string" }, depth: { type: "integer" } }, required: ["target"] });
    expect(draftProblems(filled(ctx, { title: "t" }), ctx, fields, NOW)).toEqual({
      flow: "schedule.err.flow",
      subject: "schedule.err.subject",
      project: "schedule.err.projectNeeded",
      "input.target": "schedule.err.required",
    });
    expect(draftProblems(filled(ctx, { title: "t", flow: "f", subject: "s", project: ROOT, inputs: { target: "src/", depth: "deep" } }), ctx, fields, NOW)).toEqual({ "input.depth": "schedule.err.number" });
  });

  it("a frequency's problem is the draft's, by its field", () => {
    const d = filled(SESSION, { title: "t", prompt: "hi" });
    expect(draftProblems({ ...d, freq: { ...d.freq, kind: "weekly", weekdays: [] } }, SESSION, [], NOW)).toEqual({ weekdays: "schedule.err.weekdays" });
    expect(draftProblems({ ...d, freq: { ...d.freq, kind: "once", date: "2026-11-15", time: "09:00" } }, SESSION, [], NOW)).toEqual({ when: "schedule.err.past" });
  });

  it("is created in the opener's space, a task's shown where the opener lists sessions", () => {
    expect(createParams(filled(SESSION, { title: " Morning ", prompt: " hi ", freq: { ...initialDraft(SESSION, NOW).freq, kind: "workdays", time: "09:00" } }), SESSION)).toEqual({
      space: "sessions",
      title: "Morning",
      timing: { kind: "weekly", weekdays: [1, 2, 3, 4, 5], time: "09:00" },
      time_zone: "Asia/Shanghai",
      target: { kind: "session", instance: "main", session_id: "s1", prompt: "hi" },
      missed: "catch_up_latest",
    });
    const ctx: EditorContext = { kinds: ["task"], project: ROOT, space: "sessions", shownIn: "sessions" };
    const fields = inputFields({ type: "object", properties: { target: { type: "string" } } });
    const task = createParams(filled(ctx, { title: "Nightly", flow: "example-review", subject: "the repo", inputs: { target: "src/" }, concurrency: "parallel", workspace: "worktree" }), ctx, fields);
    expect(task).toMatchObject({ space: "sessions", concurrency: "parallel" });
    expect(task.target).toEqual({ kind: "task", flow: "example-review", subject: "the repo", inputs: { target: "src/" }, project_root: ROOT, workspace: "worktree", shown_in: "sessions" });
  });
});

describe("an edit", () => {
  it("opens on the schedule as it is", () => {
    const s = schedule({ timing: { kind: "cron", expr: "30 8 15 * *" }, missed: "skip" });
    expect(initialDraft({ existing: s, kinds: ["session"] }, NOW)).toMatchObject({ title: "Morning check", zone: "Asia/Shanghai", kind: "session", prompt: "Say exactly: fixture ready", missed: "skip", freq: { kind: "monthly", day: 15, time: "08:30" } });
  });

  it("sends only what changed, at the revision it read; an every left alone is not sent again", () => {
    const s = schedule({ timing: { kind: "every", minutes: 30, from: 123 } });
    const ctx: EditorContext = { existing: s, kinds: ["session"] };
    const same = initialDraft(ctx, NOW);
    expect(updateParams(same, ctx, s)).toEqual({ id: "sch-1", revision: 3 });
    expect(changesAnything(updateParams(same, ctx, s))).toBe(false);
    expect(updateParams({ ...same, title: "Renamed" }, ctx, s)).toEqual({ id: "sch-1", revision: 3, title: "Renamed" });
    expect(updateParams({ ...same, freq: { ...same.freq, every: 2, everyUnit: "hours" } }, ctx, s)).toEqual({ id: "sch-1", revision: 3, timing: { kind: "every", minutes: 120 } });
    expect(updateParams({ ...same, prompt: "other" }, ctx, s).target).toEqual({ kind: "session", instance: "main", session_id: "s1", prompt: "other" });
  });

  it("a new session reused so far stays reused while its mode and project do", () => {
    const s = schedule({ target: { kind: "new_session", mode: "new_once_then_reuse", project_root: ROOT, prompt: "hi", session: { instance: "main", session_id: "s9" } } });
    const ctx: EditorContext = { existing: s, kinds: ["new_session"] };
    const d = initialDraft(ctx, NOW);
    expect(targetOf(d, ctx)).toMatchObject({ session: { session_id: "s9" } });
    expect(updateParams(d, ctx, s)).toEqual({ id: "sch-1", revision: 3 });
    expect(targetOf({ ...d, mode: "new_each" }, ctx)).not.toHaveProperty("session");
    expect(targetOf({ ...d, project: "/work/beta" }, ctx)).not.toHaveProperty("session");
  });

  it("what another device changed between two readings, by field", () => {
    const before = schedule();
    expect(changedFields(before, { ...before, revision: 4 })).toEqual([]);
    expect(changedFields(before, { ...before, revision: 4, title: "x", enabled: false, timing: { kind: "daily", time: "10:00" } })).toEqual(["title", "enabled", "timing"]);
    expect(changedFields(schedule({ timing: { kind: "every", minutes: 5, from: 1 } }), schedule({ timing: { kind: "every", minutes: 5, from: 2 } }))).toEqual([]);
  });

  it("kept over a newer revision: what this draft changed stays, what it left alone is theirs", () => {
    const before = schedule();
    const after: typeof before = { ...before, revision: 4, timing: { kind: "daily", time: "10:00" }, missed: "skip" };
    const ctx: EditorContext = { existing: before, kinds: ["session"] };
    const mine = { ...initialDraft(ctx, NOW), title: "Renamed", missed: "catch_up_latest" as const };
    const kept = rebase(mine, ctx, before, after, NOW);
    expect(kept).toMatchObject({ title: "Renamed", freq: { kind: "daily", time: "10:00" }, missed: "skip" });
    expect(updateParams(kept, ctx, after)).toEqual({ id: "sch-1", revision: 4, title: "Renamed" });
  });
});
