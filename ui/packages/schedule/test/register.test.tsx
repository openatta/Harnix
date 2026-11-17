/**
 * What the package registers, and only through other packages' points: the session menu's
 * "Schedule…" (not for a session another extension owns), the clocks on a session's row and a
 * task's, another way to start a flow, a project's block at 40, the home page's block at 50; each
 * dropped quietly where nobody defines its point. And the lists those draw: a session's, a
 * project's, the home page's with the models' jobs; run now, pause, delete, and the deliveries opened.
 */

import type { ReactNode } from "react";
import { act, cleanup, fireEvent, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { ScheduleList } from "@harnix/core";
import { Registry, setLocale, type ActionTarget } from "@harnix/react";

import { AllSchedules, STRINGS, STRINGS_OWNER, ScheduleStore, extension, register } from "../src/index";
import { ROOT, schedule } from "./fixtures";
import { drawn, fakeHost, type Fake } from "./host";

setLocale("en");
afterEach(() => {
  cleanup();
  localStorage.clear();
});

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const POINTS = [
  ["harnix-app", "harnix-app.session.badge", "render"],
  ["harnix-app", "harnix-app.project.panels", "render"],
  ["harnix-app", "harnix-app.home.panels", "render"],
  ["flow", "flow.task.badge", "render"],
  ["flow", "flow.start.action", "register"],
] as const;

function registered(fake: Fake, defined = true) {
  const r = new Registry();
  if (defined) for (const [owner, name, kind] of POINTS) r.activate(owner, () => r.definePoint(name, { kind }));
  r.activate(extension, () => register(r, fake.host));
  r.settle();
  return r;
}

const LIST: ScheduleList = {
  schedules: [
    schedule(),
    schedule({ id: "sch-2", title: "Paused one", enabled: false, next_fire: undefined }),
    schedule({ id: "sch-3", title: "Nightly review", target: { kind: "task", flow: "example-review", subject: "the repo", project_root: ROOT }, project: ROOT, space: "sessions" }),
  ],
  model: [{ instance: "main", id: "job-1", session_id: "s1", cron: "0 1 * * *", prompt: "check the build", recurring: true, next_fire: Date.parse("2026-11-17T01:00:00Z") }],
  unreachable: ["other"],
};

const target = (sessionId: string, space = "sessions"): ActionTarget => ({ kind: "session", session: { instance: "main", sessionId }, space });

describe("the registration", () => {
  it("is the schedule extension's interface, its words under its own owner, the same in both languages", () => {
    expect(extension).toBe("schedule");
    const keys = (d: Record<string, string>) => Object.keys(d).sort();
    expect(keys(STRINGS.en)).toEqual(keys(STRINGS["zh-CN"]));
    expect(Object.keys(STRINGS.en).every((k) => k.startsWith(`${STRINGS_OWNER}.`))).toBe(true);
    const holes = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join();
    expect(Object.keys(STRINGS.en).filter((k) => holes(STRINGS.en[k]!) !== holes(STRINGS["zh-CN"][k]!))).toEqual([]);
    expect(Object.values(STRINGS["zh-CN"]).every((v) => v.trim())).toBe(true);
  });

  it("fills each point where it is defined; nothing in the diagnostics where it is not", () => {
    const fake = fakeHost();
    const r = registered(fake);
    expect(r.list("harnix-app.project.panels").map((p) => [p.id, (p as { order?: number }).order])).toEqual([["schedule.project", 40]]);
    expect(r.list("harnix-app.home.panels").map((p) => [p.id, (p as { order?: number }).order, (p as { wide?: boolean }).wide])).toEqual([["schedule.all", 50, true]]);
    expect(r.list("flow.start.action").map((p) => p.id)).toEqual(["schedule.task"]);
    expect(r.list("harnix-app.session.badge").map((p) => p.id)).toEqual(["schedule.clock"]);
    expect(r.list("flow.task.badge").map((p) => p.id)).toEqual(["schedule.next"]);
    expect(r.at("session.action").map((a) => a.id)).toEqual(["schedule.session"]);
    expect(r.icon("schedule.clock")).toBeTruthy();
    expect(r.refusals()).toEqual([]);
    const bare = registered(fakeHost(), false);
    expect(bare.refusals()).toEqual([]);
    expect(bare.at("session.action").map((a) => a.id)).toEqual(["schedule.session"]);
  });

  it("the session menu's item: on a session of the space's own, not one another extension shows here", async () => {
    const fake = fakeHost(() => ({}), { sessions: [{ session_id: "s1", instance: "main", space: "sessions", name: "Chat" }, { session_id: "m1", instance: "main", space: "flow", exposed_from: "flow" }] });
    const r = registered(fake);
    const action = r.at("session.action").find((a) => a.id === "schedule.session")!;
    expect(action.label(target("s1"))).toBe("Schedule…");
    expect(action.match(target("s1", "projects"))).toBe(true);
    expect(action.match(target("m1", "projects"))).toBe(false);
    await action.run(target("s1", "projects"));
    fake.push("schedule.list", LIST);
    const page = drawn(fake, fake.dialogs[0]!(() => undefined));
    await settle();
    const pane = page.getByTestId("schedule-session");
    // Its schedules, on or not, and the model's job in it; not the project's task.
    expect(within(pane).getAllByTestId("schedule-row").map((el) => el.dataset.schedule)).toEqual(["sch-1", "sch-2"]);
    expect(within(pane).getAllByTestId("schedule-model-row").map((el) => el.dataset.job)).toEqual(["job-1"]);
    // A new one: the editor, a turn in this session, in the session's own space.
    fireEvent.click(page.getByTestId("schedule-new"));
    const editor = drawn(fake, fake.dialogs[1]!(() => undefined));
    await settle();
    expect(editor.getByTestId("schedule-target").textContent).toBe("A turn in this session");
    expect(editor.getByTestId("schedule-session-target").textContent).toContain("Chat");
  });

  it("the clock on a session's row while a schedule that is on turns to it, or a model's job is in it", async () => {
    const fake = fakeHost();
    const r = registered(fake);
    const badge = r.list<{ render(s: { instance: string; sessionId: string }): ReactNode }>("harnix-app.session.badge")[0]!;
    const page = drawn(
      fake,
      <>
        <div data-row="s1">{badge.render({ instance: "main", sessionId: "s1" })}</div>
        <div data-row="s2">{badge.render({ instance: "main", sessionId: "s2" })}</div>
      </>,
    );
    await settle();
    expect(page.queryByTestId("session-schedule-badge")).toBeNull();
    fake.push("schedule.list", LIST);
    await settle();
    const clocks = page.getAllByTestId("session-schedule-badge");
    expect(clocks).toHaveLength(1);
    expect(clocks[0]!.closest("[data-row]")?.getAttribute("data-row")).toBe("s1");
    // The schedule that is on, and the model's job; the paused one does not count.
    expect(clocks[0]!.dataset.count).toBe("2");
    expect(clocks[0]!.getAttribute("title")).toContain("Morning check");
    fake.push("schedule.list", { ...LIST, schedules: LIST.schedules.map((s) => ({ ...s, enabled: false, next_fire: undefined })), model: [] });
    await settle();
    expect(page.queryByTestId("session-schedule-badge")).toBeNull();
  });

  it("a task's next time on its row, while the schedule that started it is on", async () => {
    const fake = fakeHost();
    const r = registered(fake);
    const badge = r.list<{ render(run: unknown): ReactNode }>("flow.task.badge")[0]!;
    fake.push("schedule.list", LIST);
    const page = drawn(
      fake,
      <>
        {badge.render({ id: "r1", trigger: { kind: "schedule", schedule: "sch-3", fire_at: 1 } })}
        {badge.render({ id: "r2", trigger: { kind: "manual", by: "device:d1" } })}
        {badge.render({ id: "r3", trigger: { kind: "schedule", schedule: "sch-2", fire_at: 1 } })}
      </>,
    );
    await settle();
    const next = page.getAllByTestId("task-schedule-next");
    expect(next.map((el) => el.dataset.schedule)).toEqual(["sch-3"]);
    expect(next[0]!.textContent).toMatch(/^next /);
  });

  it("another way to start a flow: the editor on a task, what the dialog had carried over", async () => {
    const fake = fakeHost((method) => (method === "flow.defs" ? [{ name: "example-review", title: "Example: review", valid: true }] : {}));
    const r = registered(fake);
    const action = r.list<{ label(): string; run(d: unknown): Promise<void> }>("flow.start.action")[0]!;
    expect(action.label()).toBe("Schedule this flow…");
    await action.run({ flow: "example-review", subject: "the repo", title: "Nightly", project_root: ROOT, shown_in: "sessions", workspace: "worktree" });
    const page = drawn(fake, fake.dialogs[0]!(() => undefined));
    await settle();
    expect(page.getByTestId("schedule-target").textContent).toBe("A task");
    expect(page.getByTestId("schedule-project-fixed").textContent).toBe(ROOT);
    expect((page.getByTestId("schedule-subject") as HTMLTextAreaElement).value).toBe("the repo");
    expect((page.getByTestId("schedule-task-title") as HTMLInputElement).value).toBe("Nightly");
    expect((page.getByTestId("schedule-flow") as HTMLSelectElement).value).toBe("example-review");
    expect((page.getByTestId("schedule-worktree") as HTMLInputElement).checked).toBe(true);
    // From a flow's detail: no project yet, asked for.
    await action.run({ flow: "example-review" });
    const bare = drawn(fake, fake.dialogs[1]!(() => undefined));
    await settle();
    expect(bare.getByTestId("schedule-project")).toBeDefined();
  });

  it("a project's block: its schedules, run now said in the banner, a new one in the project", async () => {
    const fake = fakeHost((method) => (method === "schedule.runNow" ? { id: "rc1", scheduled_at: 1, manual: true, outcome: "skipped", reason: "min_interval" } : {}));
    const r = registered(fake);
    const panel = r.list<{ render(c: unknown): ReactNode }>("harnix-app.project.panels")[0]!;
    fake.push("schedule.list", LIST);
    const page = drawn(fake, panel.render({ root: `${ROOT}/`, space: "sessions" }));
    await settle();
    const rows = page.getAllByTestId("schedule-row");
    expect(rows.map((el) => el.dataset.schedule)).toEqual(["sch-3"]);
    expect(within(rows[0]!).getByTestId("schedule-when").textContent).toContain("Daily at 09:00");
    fireEvent.click(within(rows[0]!).getByTestId("schedule-run-now"));
    await settle();
    expect(fake.calls.find(([m]) => m === "schedule.runNow")?.[1]).toEqual({ id: "sch-3" });
    expect(fake.notices.at(-1)![0]).toBe("«Nightly review» was not delivered: less than a minute after the last");
    fireEvent.click(page.getByTestId("schedule-new"));
    const editor = drawn(fake, fake.dialogs[0]!(() => undefined));
    await settle();
    expect(editor.getByTestId("schedule-target-new_session").getAttribute("aria-pressed")).toBe("true");
    expect(editor.getByTestId("schedule-project-fixed").textContent).toBe(`${ROOT}/`);
  });

  it("every schedule at once, with its state and source, filtered; the models' jobs; what could not be read", async () => {
    const receipts = [
      { id: "rc2", scheduled_at: Date.parse("2026-11-16T01:00:00Z"), delivered_at: Date.parse("2026-11-16T01:00:01Z"), manual: true, outcome: "delivered", target: { instance: "main", session_id: "s1", created: false } },
      { id: "rc1", scheduled_at: Date.parse("2026-11-15T01:00:00Z"), manual: false, outcome: "skipped", reason: "missed" },
    ];
    const fake = fakeHost((method) => (method === "schedule.history" ? { receipts, total: 2 } : {}), { front: "projects" });
    const r = registered(fake);
    const store = new ScheduleStore(fake.host);
    fake.push("schedule.list", LIST);
    const page = drawn(fake, <AllSchedules host={fake.host} r={r} store={store} />);
    await settle();
    const rows = page.getAllByTestId("schedule-row");
    expect(rows.map((el) => [el.dataset.schedule, el.dataset.state, el.dataset.source])).toEqual([
      ["sch-1", "on", "user"],
      ["sch-3", "on", "user"],
      ["sch-2", "paused", "user"],
    ]);
    expect(page.getAllByTestId("schedule-model-row").map((el) => el.dataset.source)).toEqual(["model"]);
    expect(page.getByTestId("schedule-unreachable").textContent).toContain("other");
    fireEvent.change(page.getByTestId("schedule-filter-state"), { target: { value: "paused" } });
    expect(page.getAllByTestId("schedule-row").map((el) => el.dataset.schedule)).toEqual(["sch-2"]);
    fireEvent.change(page.getByTestId("schedule-filter-state"), { target: { value: "" } });
    fireEvent.change(page.getByTestId("schedule-filter-text"), { target: { value: "nightly" } });
    expect(page.getAllByTestId("schedule-row").map((el) => el.dataset.schedule)).toEqual(["sch-3"]);
    fireEvent.change(page.getByTestId("schedule-filter-text"), { target: { value: "" } });

    // Paused and resumed, deleted after asking; the model's job deleted after asking.
    const first = page.getAllByTestId("schedule-row")[0]!;
    fireEvent.click(within(first).getByTestId("schedule-toggle"));
    await settle();
    expect(fake.calls.find(([m]) => m === "schedule.disable")?.[1]).toEqual({ id: "sch-1" });
    fireEvent.click(within(first).getByTestId("schedule-delete"));
    await settle();
    expect(fake.confirms.at(-1)).toContain("Morning check");
    expect(fake.calls.find(([m]) => m === "schedule.delete")?.[1]).toEqual({ id: "sch-1" });
    fireEvent.click(page.getByTestId("schedule-model-delete"));
    await settle();
    expect(fake.calls.find(([m]) => m === "schedule.model.delete")?.[1]).toEqual({ instance: "main", id: "job-1" });

    // The deliveries: newest first, the reason said, the session a delivery went to opened where it is listed.
    fireEvent.click(within(first).getByTestId("schedule-history-toggle"));
    await settle();
    const history = within(first).getByTestId("schedule-history");
    expect(history.dataset.total).toBe("2");
    expect(within(history).getAllByTestId("schedule-receipt").map((el) => el.dataset.outcome)).toEqual(["delivered", "skipped"]);
    expect(history.textContent).toContain("missed while Harnix was not running");
    fireEvent.click(within(history).getByTestId("schedule-receipt-open"));
    await settle();
    expect(fake.opened.at(-1)).toEqual([{ instance: "main", sessionId: "s1" }, { switchSpace: true }]);
  });

  it("withdrawn: the topic is no longer followed", async () => {
    const fake = fakeHost();
    const r = new Registry();
    r.activate(extension, () => register(r, fake.host));
    r.settle();
    r.withdraw(extension);
    expect(r.at("session.action")).toEqual([]);
  });
});
