/**
 * A flow as a task in a project, in `harnix-testapp`: where it works, what it may do without a
 * person, where its deliverables and its progress go.
 *
 * - `demo-task` (`guard: auto`, the default): the `write` station's own session writes `notes.txt`
 *   into the project — no one is asked — and the `check` station's command gate finds it there,
 *   with `FLOW_WORKSPACE` the project and `FLOW_CONTEXT_FILE` in the process directory.
 * - `demo-outside` (`auto`) and `demo-outside-strict` (`strict`): the `outside` station's session
 *   writes `/tmp/harnix-flow-outside-probe.txt` under an unscoped `allow Write` — refused, saying
 *   why, under `auto`; held for a person under `strict`.
 * - `demo-reserved` (`auto`): the `reserved` station's session writes `.harnix/notes.md` under an
 *   `allow Write` over `${workspace}/**` — refused, saying why: the project's data directory is the
 *   application's, outside what a task works in.
 *
 * - `demo-task` with `workspace: worktree`, in a project made a git repository: the same write lands
 *   in its worktree out of the project, on `harnix/flow/<run>`; merged, it is in the project;
 *   discarded, gone;
 *   kept, on the branch. A project that is no git repository cannot start one.
 *
 * Session stations replay `flow-<station>`: `flow-write` calls `Write {file_path: notes.txt}` (a
 * relative path: the session's project) then `flow_submit {written: true}`; `flow-outside` calls
 * `Write` on the path above, then `flow_submit {tried: true}`; `flow-reserved` calls `Write {file_path:
 * .harnix/notes.md}`, then `flow_submit {tried: true}`.
 */

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { type Backend, type TestClient, connect, expect, test } from "@harnix/testkit";

import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:flow", "@layer:api"] };
const INVALID = -32602;
const PROBE = "/tmp/harnix-flow-outside-probe.txt";

type View = { record: any; unmet: Record<string, any[]> };

let backend: Backend;
let api: TestClient;
let project: string;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startTestApp();
});
test.afterAll(async () => { await backend?.dispose(); });
test.beforeEach(async () => {
  api = await connect(backend);
  project = realpathSync(mkdtempSync(join(tmpdir(), "harnix-task-")));
  rmSync(PROBE, { force: true });
});
test.afterEach(() => {
  api?.close();
  // Retried: the engine may still be writing under the project (its part of `.harnix/`) as it goes.
  rmSync(project, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
});

async function start(flow: string, extra: Record<string, unknown> = {}): Promise<string> {
  const { run } = await api.call("flow.start", { flow, subject: "repo", project_root: project, ...extra });
  return run;
}

async function until(run: string, ok: (v: View) => boolean, describe: string): Promise<View> {
  const sub = await api.call("harnix.topics.subscribe", { topic: "flow.run", params: { run } });
  if (sub.value && ok(sub.value)) return sub.value;
  const frame = await api.waitFor(
    (f) => f.method === "harnix.topic" && f.params.topic === "flow.run" && f.params.params?.run === run && ok(f.params.value),
    { timeout: 60_000, describe: `${run}: ${describe}` },
  );
  return frame.params.value;
}

const station = (v: View, id: string) => v.record.stations[id];
const ref = (s: string) => {
  const [instance, session_id] = s.split("/");
  return { instance, session_id };
};
const toolResults = (messages: any[]) =>
  messages.flatMap((m: any) => (Array.isArray(m.content) ? m.content : [])).filter((b: any) => b.type === "tool_result");

test("a task in a project runs two stations through with no one asked, and writes into the project", TAGS, async () => {
  const run = await start("demo-task");
  // Never waiting for an approval: it finishes on its own.
  const done = await until(run, (v) => v.record.status !== "active" || v.record.paused || station(v, "write")?.tasks?.[0]?.asking, "the run finishes");
  expect(station(done, "write").tasks[0].asking).toBeUndefined();
  expect(done.record.status).toBe("finished");
  expect(station(done, "write").tasks[0].output).toEqual({ written: true });

  // The agent's write landed in the project; the gate found it there.
  expect(readFileSync(join(project, "notes.txt"), "utf8")).toContain("harnix-task-notes");
  const gate = station(done, "check").checks.find((c: any) => c.kind === "gate");
  expect(gate).toMatchObject({ status: "pass" });
  expect(gate.detail).toContain("notes found");

  // The session worked in the project, and the protocol said where things go.
  const session = station(done, "write").tasks[0].session as string;
  const info = await api.call("engine.session.get", ref(session));
  expect(realpathSync(info.project_root)).toBe(project);
  const { messages } = await api.call("engine.session.history", ref(session));
  const said = JSON.stringify(messages);
  expect(said).toContain("work freely inside the project");
  expect(said).toContain(done.record.output_dir);
  expect(said).not.toContain("Refused unless a permission allows them");
  expect(toolResults(messages).some((b: any) => b.is_error)).toBe(false);
});

test("a task's deliverables go in flows/<flow>-<date>01, its progress in .harnix/runs, which git ignores", TAGS, async () => {
  const run = await start("demo-task");
  const done = await until(run, (v) => v.record.status === "finished", "the run finishes");
  const out: string = done.record.output_dir;
  expect(out).toMatch(new RegExp(`^${project}/flows/demo-task-\\d{8}01$`));
  expect(existsSync(out)).toBe(true);
  // The next one that day is numbered on.
  const second = await start("demo-task");
  const next = (await api.call("flow.get", { run: second })).record.output_dir as string;
  expect(next).toBe(out.replace(/01$/, "02"));
  await api.call("flow.cancel", { run: second, reason: "numbered" });

  // The process directory: the run as it stands, the pinned definition, each station's output, the
  // command gate's context and log.
  const dir = join(project, ".harnix", "runs", run);
  const task = JSON.parse(readFileSync(join(dir, "task.json"), "utf8"));
  expect(task).toMatchObject({ run, flow: "demo-task", subject: "repo", status: "finished", output_dir: out, trigger: { kind: "manual" } });
  expect(task.stations.map((s: any) => [s.id, s.status])).toEqual([["write", "passed"], ["check", "passed"]]);
  expect(readFileSync(join(dir, "flow.yaml"), "utf8")).toContain("name: demo-task");
  expect(JSON.parse(readFileSync(join(dir, "stations", "write", "output.json"), "utf8"))).toEqual({ written: true });
  expect(JSON.parse(readFileSync(join(dir, "stations", "check", "context.json"), "utf8")).run).toBe(run);
  expect(readFileSync(join(dir, "stations", "check", "gate-task-check.log"), "utf8")).toContain("notes found");
  // `runs/` is Harnix's one line; the engine adds its own to the same file when it writes there.
  // The project's own flows (`flows/`) are committed, and worktrees are not in the project at all.
  const ignored = readFileSync(join(project, ".harnix", ".gitignore"), "utf8").split("\n");
  expect(ignored).toContain("runs/");
  // The engine's are what is this machine's and this user's: its local data, all in `state/`, and one
  // user's own configuration beside the shared settings. Its worktrees are not here at all.
  expect(ignored).toEqual(expect.arrayContaining(["state/", "settings.local.json", "mcp.local.json"]));
  expect(ignored).not.toContain("memory/");
  expect(ignored).not.toContain("scratchpad/");
  expect(ignored).not.toContain("worktrees/");
  expect(ignored).not.toContain("flows/");
  // Nothing else in the project: the engine's project tier is in the same `.harnix/`.
  expect(readdirSync(project).sort()).toEqual([".harnix", "flows", "notes.txt"]);
});

test("a task's output directory: the start's own over the flow's, never outside the project, and an existing .gitignore is added to", TAGS, async () => {
  mkdirSync(join(project, ".harnix"));
  writeFileSync(join(project, ".harnix", ".gitignore"), "# mine");
  const run = await start("demo-task", { output_dir: "out/${flow}-${title}", title: "a/b" });
  expect((await api.call("flow.get", { run })).record.output_dir).toBe(join(project, "out", "demo-task-a-b"));
  const lines = readFileSync(join(project, ".harnix", ".gitignore"), "utf8").split("\n");
  expect(lines[0]).toBe("# mine");
  expect(lines).toContain("runs/");
  await api.call("flow.cancel", { run, reason: "seen" });
  for (const output_dir of ["../elsewhere", "/tmp/elsewhere", "out/${run}"]) {
    const e = await api.refused("flow.start", { flow: "demo-task", subject: "repo", project_root: project, output_dir });
    expect(e?.code, output_dir).toBe(INVALID);
  }
  expect((await api.call("flow.list", { project })).length).toBe(1);
});

test("under auto a write outside the project is refused, not asked, and the task goes on", TAGS, async () => {
  const run = await start("demo-outside");
  const done = await until(run, (v) => v.record.status !== "active" || !!station(v, "outside")?.tasks?.[0]?.asking, "the run finishes");
  expect(station(done, "outside").tasks[0].asking).toBeUndefined();
  expect(done.record.status).toBe("finished");
  expect(existsSync(PROBE)).toBe(false);
  const { messages } = await api.call("engine.session.history", ref(station(done, "outside").tasks[0].session));
  const refused = toolResults(messages).find((b: any) => JSON.stringify(b).includes("outside the task's project"));
  expect(refused, JSON.stringify(toolResults(messages)).slice(0, 600)).toBeTruthy();
  expect(refused.is_error).toBe(true);
});

test("a write into the project's .harnix is refused under auto, even under an allow over the whole workspace", TAGS, async () => {
  const run = await start("demo-reserved");
  const done = await until(run, (v) => v.record.status !== "active" || !!station(v, "reserved")?.tasks?.[0]?.asking, "the run finishes");
  expect(station(done, "reserved").tasks[0].asking).toBeUndefined();
  expect(done.record.status).toBe("finished");
  expect(existsSync(join(project, ".harnix", "notes.md"))).toBe(false);
  const { messages } = await api.call("engine.session.history", ref(station(done, "reserved").tasks[0].session));
  const refused = toolResults(messages).find((b: any) => JSON.stringify(b).includes("the project's data directory"));
  expect(refused, JSON.stringify(toolResults(messages)).slice(0, 600)).toBeTruthy();
  expect(refused.is_error).toBe(true);
});

test("under strict the same write is held for a person, as in 0.4.4", TAGS, async () => {
  const run = await start("demo-outside-strict");
  const held = await until(run, (v) => !!station(v, "outside")?.tasks?.[0]?.asking, "the write is held");
  expect(held.record.def.guard).toBe("strict");
  expect(station(held, "outside").tasks[0].asking).toMatchObject({ tool: "Write" });
  expect(station(held, "outside").tasks[0].asking.reason).toContain("a person decides");
  const summary = (await api.call("flow.list", { flow: "demo-outside-strict" })).find((s: any) => s.id === run);
  expect(summary.phase).toMatchObject({ state: "waiting", reason: "approval", stations: ["outside"] });
  expect(existsSync(PROBE)).toBe(false);
  await api.call("flow.cancel", { run, reason: "seen" });
});

// ── a task in a worktree ─────────────────────────────────────────────────────

/** The project made a git repository with one commit (`README.md`), its own identity. */
function gitProject(): void {
  const git = (...args: string[]) => execFileSync("git", args, { cwd: project, stdio: "pipe" }).toString().trim();
  git("init", "-q", "-b", "main");
  git("config", "user.name", "T");
  git("config", "user.email", "t@example.com");
  git("config", "commit.gpgsign", "false");
  writeFileSync(join(project, "README.md"), "a project\n");
  git("add", "-A");
  git("commit", "-q", "-m", "first");
}

const gitIn = (dir: string, ...args: string[]) => execFileSync("git", args, { cwd: dir, stdio: "pipe" }).toString().trim();

test("a worktree task writes in its worktree, not the project's tree; merged, the file is in the project", TAGS, async () => {
  gitProject();
  expect(await api.call("flow.workspace_info", { project_root: project })).toMatchObject({ git: true, branch: "main" });
  const run = await start("demo-task", { workspace: "worktree" });
  const done = await until(run, (v) => v.record.status !== "active" || !!station(v, "write")?.tasks?.[0]?.asking, "the run finishes");
  expect(done.record.status).toBe("finished");
  const wt = done.record.worktree;
  // Out of the project (the engine's own worktrees are in the repository's `.wkts/`), in Harnix's state:
  // the flow extension's data directory, on a branch apart from the engine's `harnix/worktree-*`.
  expect(wt).toMatchObject({ branch: `harnix/flow/${run}` });
  expect(wt.path.endsWith(`/ext/flow/data/worktrees/${run}`)).toBe(true);
  expect(wt.path.startsWith(`${project}/`)).toBe(false);
  expect(wt.outcome).toBeUndefined();

  // The write landed in the worktree; the project's own tree has none of it, and shows nothing new.
  expect(readFileSync(join(wt.path, "notes.txt"), "utf8")).toContain("harnix-task-notes");
  expect(existsSync(join(project, "notes.txt"))).toBe(false);
  expect(done.record.output_dir.startsWith(`${wt.path}/flows/demo-task-`)).toBe(true);
  expect(gitIn(project, "status", "--porcelain")).toBe("?? .harnix/");
  // The session and the gate worked there; the progress is in the project's process directory.
  const info = await api.call("engine.session.get", ref(station(done, "write").tasks[0].session));
  expect(realpathSync(info.project_root)).toBe(wt.path);
  expect(station(done, "check").checks.find((c: any) => c.kind === "gate")).toMatchObject({ status: "pass" });
  expect(JSON.parse(readFileSync(join(project, ".harnix", "runs", run, "task.json"), "utf8")).status).toBe("finished");
  // A row of the project's tasks says the branch.
  expect((await api.call("flow.list", { project })).find((s: any) => s.id === run).worktree.branch).toBe(`harnix/flow/${run}`);

  const merged = await api.call("flow.worktree.merge", { run });
  expect(merged.worktree.outcome).toMatchObject({ kind: "merged", into: "main", how: "fast_forward" });
  expect(readFileSync(join(project, "notes.txt"), "utf8")).toContain("harnix-task-notes");
  expect(existsSync(wt.path)).toBe(false);
  expect(gitIn(project, "branch", "--list", "harnix/flow/*")).toBe("");
  // Decided once.
  expect((await api.refused("flow.worktree.discard", { run }))?.code).toBe(INVALID);
});

test("a worktree task discarded leaves nothing; kept, its branch holds the work; no git repository, no worktree", TAGS, async () => {
  const plain = await api.refused("flow.start", { flow: "demo-task", subject: "repo", project_root: project, workspace: "worktree" });
  expect(plain?.code).toBe(INVALID);
  expect(plain?.message).toContain("needs a git repository");
  expect(await api.call("flow.workspace_info", { project_root: project })).toMatchObject({ git: false, reason: "not_a_repository" });

  gitProject();
  const run = await start("demo-task", { workspace: "worktree" });
  // Still going: nothing to decide.
  expect((await api.refused("flow.worktree.keep", { run }))?.code).toBe(INVALID);
  const done = await until(run, (v) => v.record.status !== "active", "the run finishes");
  const discarded = await api.call("flow.worktree.discard", { run });
  expect(discarded.worktree.outcome.kind).toBe("discarded");
  expect(existsSync(done.record.worktree.path)).toBe(false);
  expect(gitIn(project, "branch", "--list", `harnix/flow/${run}`)).toBe("");
  expect(existsSync(join(project, "notes.txt"))).toBe(false);

  const kept = await start("demo-task", { workspace: "worktree" });
  const over = await until(kept, (v) => v.record.status !== "active", "the run finishes");
  expect((await api.call("flow.worktree.keep", { run: kept })).worktree.outcome.kind).toBe("kept");
  expect(existsSync(over.record.worktree.path)).toBe(false);
  expect(gitIn(project, "show", `harnix/flow/${kept}:notes.txt`)).toContain("harnix-task-notes");
  expect(existsSync(join(project, "notes.txt"))).toBe(false);
});
