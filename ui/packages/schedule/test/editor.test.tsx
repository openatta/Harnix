/**
 * The editor as a person uses it: nothing is sent while something is wrong, and what is wrong is
 * said by its field; the frequency's kinds and the weekday chips; a new schedule created with the
 * opener's target and space; a refusal said as the scheduler says it; an edit that met another
 * device's change kept and reviewed against the current revision, then saved over it.
 */

import { act, cleanup, fireEvent } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CONFLICT, INVALID_PARAMS, type Schedule } from "@harnix/core";
import { Registry, setLocale } from "@harnix/react";

import type { EditorContext } from "../src/draft";
import { ScheduleEditor } from "../src/editor";
import { registerScheduleStrings } from "../src/strings";
import { ROOT, schedule } from "./fixtures";
import { drawn, fakeHost, type Fake } from "./host";

setLocale("en");
registerScheduleStrings(new Registry());
afterEach(() => {
  cleanup();
  localStorage.clear();
});

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

async function editor(fake: Fake, ctx: EditorContext) {
  const close = vi.fn();
  const page = drawn(fake, <ScheduleEditor host={fake.host} ctx={ctx} close={close} />);
  await settle();
  return { page, close };
}

const SESSION: EditorContext = { kinds: ["session"], space: "sessions", session: { instance: "main", session_id: "s1", name: "Chat" } };

describe("the editor", () => {
  it("sends nothing while something is wrong, and says what by its field", async () => {
    const fake = fakeHost();
    const { page } = await editor(fake, SESSION);
    expect(page.getByTestId("schedule-session-target").textContent).toContain("Chat");
    fireEvent.click(page.getByTestId("schedule-save"));
    await settle();
    expect(page.getByTestId("schedule-problem-title").textContent).toBe("A title is needed");
    expect(page.getByTestId("schedule-problem-prompt").textContent).toBe("Say what to send");
    expect((page.getByTestId("schedule-save") as HTMLButtonElement).disabled).toBe(true);
    // Weekly with no day picked: the chips say so.
    fireEvent.click(page.getByTestId("schedule-freq-weekly"));
    fireEvent.click(page.getByTestId("schedule-weekday-1"));
    expect(page.getByTestId("schedule-problem-weekdays").textContent).toBe("Pick at least one day");
    expect(fake.calls.filter(([m]) => m.startsWith("schedule."))).toEqual([]);
  });

  it("creates a daily one for the session it was opened for, in that session's space", async () => {
    const made = schedule();
    const fake = fakeHost((method) => (method === "schedule.create" ? made : {}));
    const { page, close } = await editor(fake, SESSION);
    fireEvent.change(page.getByTestId("schedule-title"), { target: { value: "Morning check" } });
    fireEvent.change(page.getByTestId("schedule-prompt"), { target: { value: "Say exactly: fixture ready" } });
    fireEvent.change(page.getByTestId("schedule-time"), { target: { value: "08:15" } });
    expect(page.getByTestId("schedule-freq-summary").textContent).toBe("Daily at 08:15");
    fireEvent.change(page.getByTestId("schedule-zone"), { target: { value: "Asia/Shanghai" } });
    fireEvent.click(page.getByTestId("schedule-save"));
    await settle();
    expect(fake.calls.find(([m]) => m === "schedule.create")?.[1]).toEqual({
      space: "sessions",
      title: "Morning check",
      timing: { kind: "daily", time: "08:15" },
      time_zone: "Asia/Shanghai",
      target: { kind: "session", instance: "main", session_id: "s1", prompt: "Say exactly: fixture ready" },
      missed: "catch_up_latest",
    });
    expect(close).toHaveBeenCalledWith(made);
    // The zone is one used lately now.
    expect(JSON.parse(localStorage.getItem("harnix.schedule.zones") ?? "[]")).toEqual(["Asia/Shanghai"]);
  });

  it("each kind of frequency has its own boxes; workdays and a month's day go as a week and a cron", async () => {
    const fake = fakeHost((method) => (method === "schedule.create" ? schedule() : {}));
    const { page } = await editor(fake, SESSION);
    fireEvent.change(page.getByTestId("schedule-title"), { target: { value: "t" } });
    fireEvent.change(page.getByTestId("schedule-prompt"), { target: { value: "p" } });
    fireEvent.click(page.getByTestId("schedule-freq-every"));
    expect(page.getByTestId("schedule-every")).toBeDefined();
    fireEvent.click(page.getByTestId("schedule-freq-after"));
    expect(page.getByTestId("schedule-after-unit")).toBeDefined();
    fireEvent.click(page.getByTestId("schedule-freq-once"));
    expect(page.getByTestId("schedule-date")).toBeDefined();
    fireEvent.click(page.getByTestId("schedule-freq-workdays"));
    expect(page.getByTestId("schedule-freq-summary").textContent).toBe("Workdays at 09:00");
    fireEvent.click(page.getByTestId("schedule-freq-monthly"));
    fireEvent.change(page.getByTestId("schedule-day"), { target: { value: "15" } });
    fireEvent.click(page.getByTestId("schedule-save"));
    await settle();
    expect((fake.calls.find(([m]) => m === "schedule.create")?.[1] as { timing: unknown }).timing).toEqual({ kind: "cron", expr: "0 9 15 * *" });
  });

  it("a custom cron the scheduler refuses: its words, nothing closed", async () => {
    const refused = { code: INVALID_PARAMS, message: "`0 0 31 2 *` is never due" };
    const fake = fakeHost((method) => (method === "schedule.create" ? refused : {}));
    const { page, close } = await editor(fake, SESSION);
    fireEvent.change(page.getByTestId("schedule-title"), { target: { value: "t" } });
    fireEvent.change(page.getByTestId("schedule-prompt"), { target: { value: "p" } });
    fireEvent.click(page.getByTestId("schedule-freq-cron"));
    fireEvent.change(page.getByTestId("schedule-cron"), { target: { value: "0 0 31 2 *" } });
    fireEvent.click(page.getByTestId("schedule-save"));
    await settle();
    expect(page.getByTestId("schedule-error").textContent).toBe("`0 0 31 2 *` is never due");
    expect(close).not.toHaveBeenCalled();
  });

  it("a task in the project it was opened from: flows of the project, its inputs, shown where the opener lists sessions", async () => {
    const defs = [{ name: "example-review", title: "Example: review", valid: true, inputs: { type: "object", properties: { target: { type: "string" } }, required: ["target"] } }];
    const fake = fakeHost((method) => (method === "flow.defs" ? defs : method === "schedule.create" ? schedule() : {}));
    const { page } = await editor(fake, { kinds: ["new_session", "task"], project: ROOT, space: "sessions", shownIn: "sessions" });
    expect(page.getByTestId("schedule-project-fixed").textContent).toBe(ROOT);
    fireEvent.click(page.getByTestId("schedule-target-task"));
    await settle();
    expect(fake.calls.find(([m]) => m === "flow.defs")?.[1]).toEqual({ project: ROOT });
    fireEvent.change(page.getByTestId("schedule-flow"), { target: { value: "example-review" } });
    await settle();
    fireEvent.change(page.getByTestId("schedule-title"), { target: { value: "Nightly review" } });
    fireEvent.change(page.getByTestId("schedule-subject"), { target: { value: "the repo" } });
    fireEvent.click(page.getByTestId("schedule-save"));
    await settle();
    expect(page.getByTestId("schedule-problem-input.target").textContent).toBe("Required");
    fireEvent.change(page.getByTestId("schedule-input-target").querySelector("input")!, { target: { value: "src/" } });
    fireEvent.click(page.getByTestId("schedule-concurrency-parallel"));
    fireEvent.click(page.getByTestId("schedule-save"));
    await settle();
    expect(fake.calls.find(([m]) => m === "schedule.create")?.[1]).toMatchObject({
      space: "sessions",
      concurrency: "parallel",
      target: { kind: "task", flow: "example-review", subject: "the repo", inputs: { target: "src/" }, project_root: ROOT, shown_in: "sessions" },
    });
  });

  it("an edit that met another device's change: kept, reviewed against the current revision, then saved over it", async () => {
    const mine = schedule({ revision: 3 });
    const theirs: Schedule = { ...mine, revision: 4, timing: { kind: "daily", time: "10:00" } };
    let updates = 0;
    const fake = fakeHost((method, params) => {
      if (method === "schedule.update") return ++updates === 1 ? { code: CONFLICT, message: "changed", data: { revision: 4 } } : { ...theirs, ...(params as object), revision: 5 };
      if (method === "schedule.get") return theirs;
      return {};
    });
    const { page, close } = await editor(fake, { existing: mine, kinds: ["session"] });
    expect((page.getByTestId("schedule-title") as HTMLInputElement).value).toBe("Morning check");
    fireEvent.change(page.getByTestId("schedule-title"), { target: { value: "Morning check, renamed" } });
    fireEvent.click(page.getByTestId("schedule-save"));
    await settle();
    expect(page.getByTestId("schedule-conflict")).toBeDefined();
    expect((page.getByTestId("schedule-save") as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(page.getByTestId("schedule-conflict-keep"));
    await settle();
    const reviewed = page.getByTestId("schedule-reviewed");
    expect(reviewed.dataset.revision).toBe("4");
    expect(reviewed.textContent).toContain("when");
    // Mine kept: the title as typed.
    expect((page.getByTestId("schedule-title") as HTMLInputElement).value).toBe("Morning check, renamed");
    fireEvent.click(page.getByTestId("schedule-save"));
    await settle();
    const sent = fake.calls.filter(([m]) => m === "schedule.update").map(([, p]) => p);
    expect(sent[0]).toEqual({ id: "sch-1", revision: 3, title: "Morning check, renamed" });
    // Over theirs: their time is not undone, only my title goes.
    expect(sent[1]).toEqual({ id: "sch-1", revision: 4, title: "Morning check, renamed" });
    expect(close).toHaveBeenCalled();
  });
});
