/**
 * The recordings made against a real model (`record-fixture.mjs`), and how each was made: the message,
 * the scene, the backend it ran on. `record-fixture.mjs <name>` records one of them again the way it
 * is written here; `strict-replay.spec.ts` replays every one of them strictly, set up the same way, so
 * each request is the recorded one byte for byte.
 *
 * One whose requests cannot be the same twice says why (`varies`) and is replayed by position only, as
 * every other suite replays. Each is made in its own fixed directory (`STRICT_ROOT/<name>`), because
 * the prompt names the directories. The prompt names the operating system too, so they are recorded
 * on macOS and replay strictly there only.
 */

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import type { SessionRow, TaskRow } from "@harnix/core/protocol";
import { type Backend, type LaunchOptions, type TestClient, runTurn, sleep } from "@harnix/testkit";

// With its extension: `record-fixture.mjs` loads this under plain `node`.
import { SHOWCASE_ID, appsTools, showcasePackage } from "./apps-host.ts";

export interface Recorded {
  name: string;
  /** What the session is sent. A recording answers the message it was made with. */
  message: string;
  /** The scene it is created in; `null`: the binary's default, as a suite that names none. Default `harnix`. */
  scene?: string | null;
  /** `session.create`'s options beside the recorder (a permission mode). */
  options?: Record<string, unknown>;
  /** The project's `.agix/settings.json` (an operator's switch, such as teams). */
  settings?: Record<string, unknown>;
  /** On the binary that hosts applications, with the showcase installed. */
  apps?: boolean;
  /** Profile TOML for its backend (a scene to activate beside the default). */
  profile?: string;
  /** What it starts outlives its turn (a background sub-agent, a Monitor): it is waited for too. */
  tasks?: boolean;
  /**
   * Why its requests are not the same twice — a tool answers with a pid or a task id, which the next
   * request carries — so it is recorded here but not replayed strictly.
   */
  varies?: string;
}

const tool = (name: string) => `app__example_showcase__${name}`;
/** The `code` scene beside the default, as the suites that replay its tools run it. */
const CODE = '\n[engine]\nscenes = ["code"]\n';
/** "Call it, then say this": the shape most of them have. */
const callThen = (call: string, reply: string) => `${call} Call the tool now; do not answer in prose first. When it returns, reply with exactly: ${reply}`;

export const RECORDED: Recorded[] = [
  { name: "says-ready", message: "Say exactly: fixture ready" },
  { name: "calls-a-tool", message: "Use the Bash tool right now to run exactly: echo harnix-fixture. Call the tool. Do not answer in prose." },
  { name: "asks-permission", message: "Use the Bash tool right now to run exactly: rm -f /tmp/harnix-fixture-probe. Call the tool. Do not answer in prose." },
  {
    name: "runs-a-while",
    message: 'Use the Bash tool right now to run exactly: perl -e "select(undef, undef, undef, 4)"; echo waited. Call the tool. When it returns, reply with exactly one word: waited',
  },
  {
    name: "says-markdown",
    message:
      'Reply with exactly the following markdown, and nothing else: "Here it is:", then a sh code block containing "echo fixture ready", then a table with the header row "| name | value |" and the row "| answer | fixture ready |", then "See [the docs](https://example.com/docs)."',
  },
  {
    name: "spawns-agent",
    message:
      "Use the Agent tool exactly once to launch a general-purpose sub-agent with the description 'list files' and the prompt 'Run ls in the current directory and reply with the file names only.' When it returns, reply with one line: 'sub-agent found:' followed by the names.",
  },  {
    name: "leaves-a-process",
    varies: "the Bash call prints the pid of the process it left",
    message: callThen('Use the Bash tool to run exactly: tail -f /dev/null >/dev/null 2>&1 & echo "PID=$!"', "started"),
  },
  {
    name: "messages-a-subagent",
    tasks: true,
    varies: "the Agent call answers with the background task's id",
    message:
      "Use the Agent tool exactly once to launch a general-purpose sub-agent in the background (do not wait for it), with the description 'wait, then answer' and the prompt 'Use the Bash tool to run exactly: perl -e \"select(undef, undef, undef, 10)\"; echo waited. Then reply with exactly: sub-agent done'. Then reply with exactly: Started it in the background.",
  },
  {
    name: "runs-a-team",
    varies: "TeamCreate answers with the team's task ids",
    scene: "code",
    profile: CODE,
    settings: { execution: { team_enabled: true } },
    tasks: true,
    message:
      "Use the TeamCreate tool exactly once to run a team named ui-team with one member labelled alpha, whose prompt is 'Say exactly: alpha here'. When the team is done, reply with exactly: Team done.",
  },
  { name: "app-echo", scene: null, apps: true, message: callThen(`Use the ${tool("echo")} tool with the text showcase-echo-42.`, "The showcase said it back.") },
  { name: "app-count", scene: null, apps: true, message: callThen(`Use the ${tool("count")} tool with the key visits.`, "Counted.") },
  { name: "app-config", scene: null, apps: true, message: callThen(`Use the ${tool("config")} tool, with no arguments.`, "That is the showcase's configuration.") },
  { name: "app-read-project", scene: null, apps: true, message: callThen(`Use the ${tool("read_project")} tool with the path README.md.`, "That is the project's README.") },
  { name: "app-vetoed", scene: null, apps: true, message: callThen(`Use the ${tool("vetoed")} tool, with no arguments.`, "The showcase refused it.") },
  { name: "app-slow", scene: null, apps: true, message: callThen(`Use the ${tool("slow")} tool with steps 3 and ms 300.`, "The slow tool finished.") },
  { name: "app-slow-long", scene: null, apps: true, message: callThen(`Use the ${tool("slow")} tool with steps 100 and ms 300.`, "The slow tool finished.") },
  { name: "app-board", scene: null, apps: true, message: callThen(`Use the ${tool("board")} tool, with no arguments.`, "The showcase's board is as it said.") },
  {
    name: "app-crash",
    scene: null,
    apps: true,
    message: `Use the ${tool("crash")} tool, with no arguments, three times in a row, one call after another, whatever each one answers. Then reply with exactly: The showcase's crash tool failed three times.`,
  },
  {
    name: "app-subagent",
    scene: null,
    apps: true,
    message:
      "Use the Agent tool exactly once, with the subagent_type showcase-guide, the description 'try the showcase' and the prompt 'Call the showcase's echo tool with the text showcase-sub-7 and say what it answered.' When it returns, reply with exactly: The sub-agent used the showcase's tool.",
  },
];

/** The backend a recording is made on, or replayed on: `launch` is `startHarnix` with the run's options. */
export async function backendFor(entry: Recorded, base: LaunchOptions, launch: (options: LaunchOptions) => Promise<Backend>): Promise<Backend> {
  const own = entry.profile ? { ...base, profile: entry.profile } : base;
  const options: LaunchOptions = entry.apps
    ? {
        ...own,
        bin: appsTools().bin,
        profile: `
[apps]
enabled = true
dev = true

[apps.config."${SHOWCASE_ID}"]
greeting = "hello from the profile"
${entry.profile ?? ""}`,
      }
    : own;
  if (entry.apps) showcasePackage();
  return launch(options);
}

/**
 * The recording's session, on `backend` through `api`: what the backend needs first (the showcase,
 * the project's settings), then the session, its turn with every ask permitted, and — for one that
 * starts something lasting — that, until it is done. Answers the session and the settlement.
 */
export async function play(api: TestClient, entry: Recorded, recorder?: string): Promise<{ session_id: string; settled: any }> {
  if (entry.apps) await api.call("harnix.apps.installFile", { path: showcasePackage().path });
  if (entry.settings) {
    mkdirSync(join(api.project, ".agix"), { recursive: true });
    writeFileSync(join(api.project, ".agix", "settings.json"), JSON.stringify(entry.settings));
  }
  const scene = entry.scene === undefined ? "harnix" : entry.scene;
  const created = await api.call("session.create", {
    ...(scene ? { scene } : {}),
    project_root: api.project,
    options: { ...entry.options, ...(recorder ? { recorder: { name: recorder } } : {}) },
  });
  const session = { instance: "system", session_id: created.session_id };
  await api.call("harnix.attach", session);
  const { settled } = await runTurn(api, session, entry.message, { timeout: 240_000 });
  if (entry.tasks) await tasksDone(api, session);
  return { session_id: created.session_id, settled };
}

/**
 * Until nothing the session started is running and no turn is either — a finished task can hand its
 * result back as a turn of its own — for three looks a second apart; every ask in the meantime
 * permitted (a background sub-agent's Bash).
 */
async function tasksDone(api: TestClient, session: { instance: string; session_id: string }): Promise<void> {
  const answered = new Set<string>();
  let quiet = 0;
  for (const deadline = Date.now() + 180_000; Date.now() < deadline; ) {
    for (const f of api.events) {
      if (f.method !== "harnix.event" || f.params.event?.kind !== "prompt" || answered.has(f.params.event.prompt_id)) continue;
      answered.add(f.params.event.prompt_id);
      await api
        .call("session.respondToPrompt", { instance: f.params.instance ?? session.instance, session_id: f.params.session_id, prompt_id: f.params.event.prompt_id, decision: { type: "permit" } })
        .catch(() => {});
    }
    const { tasks } = await api.call("harnix.sessions.tasks", session);
    const { sessions } = await api.call("harnix.instances.sessions", { instance: session.instance });
    const busy =
      tasks.some((t: TaskRow) => t.status === "running" || t.status === "waiting") ||
      sessions.some((r: SessionRow) => r.session_id === session.session_id && r.running);
    quiet = busy ? 0 : quiet + 1;
    if (quiet >= 3) return;
    await sleep(1000);
  }
  throw new Error("what the session started was still running after three minutes");
}
