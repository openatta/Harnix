# Recorded model calls

Fixtures for replay. Each directory is one recorded session — the requests
that went to the model and the responses that came back — and replaying one
makes an agent's behaviour deterministic and free: no provider, no network, no
waiting, and the same tool calls every time.

Recorded with `harnix` in its ordinary mode (recording is always on — §6.4);
played back with `--replay-dir`, which is an **operator's** flag. A client
cannot ask for replay, and outside replay the fixture name means nothing.

| Fixture | What it exercises |
|---|---|
| `says-ready` | The plain path: one turn, text out, no tools |
| `calls-a-tool` | A `Bash` tool call — the tool row, its result, the details pane |
| `asks-permission` | A `Bash` call that needs permission — the ask, the broadcast, the answer |
| `spawns-agent` | An `Agent` call: a sub-agent that is a task of its session, not a session in the list |
| `sets-a-goal` | A `GoalCreate` call (objective "Keep the fixture tests green"), then "Goal set.": a session with a goal. The engine opens the next round itself; the recording has nothing for it, the round fails and the goal is left ongoing but disarmed. Made from `calls-a-tool` by hand: the tool, its input and the text differ |
| `runs-a-team` | Two sessions: the parent calls `TeamCreate` (team `ui-team`, one stage, member `alpha` — a `code`-scene tool, shown only with `execution.team_enabled`), the member says "alpha here", the parent "Team done.". Replay binds live sessions to recorded ones in the order they first call, so the member is a session of its own here. Made by hand from `calls-a-tool`'s request and blobs |
| `messages-a-subagent` | Two sessions: the parent starts a background `Agent`, which runs ten seconds of Bash (`perl`, as in `runs-a-while`) and then says "sub-agent done"; the parent says "Started it in the background.". A sub-agent still in its turn, to be told something (`messageTask`). Made the same way |
| `leaves-a-process` | A `Bash` call that leaves a `tail -f /dev/null` in the background and prints its pid (ipdrun-07). Made from `calls-a-tool` by hand: only the tool input and the text differ |
| `calls-a-native-tool` | A call to `ProbeFixed`, the tool `harnix-testapp`'s native scene contributes (ipdrun-10). Made the same way |
| `flow-analyze` | The flow module's `analyze` station (xops-01): one call to `flow_submit` with `{findings: [one low finding]}`, then a line of text. Made from `calls-a-native-tool` by hand; `harnix-testapp` names a session task's recording `flow-<station>` |
| `flow-report` | The same for the `report` station's `summary` task: `flow_submit` with `{summary}` |
| `flow-propose` | One turn of four calls: `TodoWrite` (which `harnix-testapp` guards as a write, so the flow refuses it), `flow_propose {kind: rework, to: propose}`, `flow_submit {done: true}`, a line of text. Made from `calls-a-native-tool` by hand |
| `flow-shared` | Two turns in one session (`per_run`): each calls `flow_submit` with its own `step` and says a line; nothing between them — the memory extraction after a turn is an auxiliary call, matched by its purpose, and a recording without one makes the engine skip it. Made the same way |
| `flow-local`, `flow-remote` | xops-02: a station's session on a template instance (in process / spawned) hands in by structured output — one call to `StructuredOutput` with `{findings: [one low finding]}`, then a line of text. Made the same way, with `StructuredOutput` as the tool |
| `flow-guarded`, `flow-bypassed` | xops-02: `TodoWrite` (refused by the session's `deny` permission rule — for `flow-bypassed` on an instance set to `bypassPermissions`), then `StructuredOutput {done: true}`, then a line of text |
| `flow-author-unfocused` | xops-02: a `flow-author` conversation calls `flow_update` with no flow selected (and is told to ask for a selection) |
| `flow-author-edit` | xops-02: three `flow-author` turns — `flow_schema`, `flow_validate`, `flow_create` · `flow_show`, `flow_update`, `flow_dry_run` · `flow_delete` — back to back, as in `flow-shared` |
| `flow-ui-edit` | xops-02 §7, the flow space in the browser: one `flow-author` turn — `flow_show`, then `flow_update {flow: example-review, yaml}` with the empty application's `example-review` retitled "Example: reviewed in conversation" and then a line of text. Made from `flow-author-unfocused` by hand |
| `runs-a-while` | A `Bash` call that waits four seconds (in `perl`: the engine refuses a `sleep` of 2 s or more): a turn that is still running when something is steered into it (ipdrun-13). Made the same way |
| `many-turns` | `says-ready`'s one call sixty times over: a session long enough to compact (eight turns), whichever call comes next — a turn, a memory extraction, a compaction's summary — answered the same. Made by `python3` from `says-ready`, sequence numbers renumbered |
| `streams-output` | A `Monitor` call (a `code`-scene tool) whose command prints a line every quarter second for ten seconds: a running task whose output grows. Made from `leaves-a-process` by hand: the tool's name and input differ |
| `app-echo`, `app-count`, `app-config`, `app-read-project`, `app-vetoed` | The showcase application (`examples/apps/showcase`): one call to its tool `app__example_showcase__<tool>` — `echo {text: showcase-echo-42}`, `count {key: visits}`, `config {}`, `read_project {path: README.md}`, `vetoed {}` — then a line of text. Made by a script from `calls-a-native-tool`'s request blobs: only the tool, its input and the text are its own |
| `app-slow`, `app-slow-long` | The same for `slow {steps: 3, ms: 300}` and `slow {steps: 100, ms: 300}`: a tool that reports progress, and one still working when its turn is cancelled |
| `app-subagent` | Two sessions: the parent calls `Agent` with the showcase's own agent type (`showcase-guide`), the sub-agent calls `app__example_showcase__echo {text: showcase-sub-7}` and says a line, the parent says "The sub-agent used the showcase's tool.". Made the same way |
| `app-board` | The showcase's `board {}`: its tool that asks the backend for its state with `host.call` (`tests/live/api/apps-backend.spec.ts`), then a line of text. Made from `app-echo` by a script: only the tool, its input and the text differ |
| `app-backend-session` | One call, text only: "The showcase's backend started this conversation." — the session the showcase's backend creates and prompts (`session.start`). Made from `app-echo`'s first request by a script |
| `app-mcp-echo` | A spawned daemon's session calls `mcp__app-example_showcase__echo {text: showcase-over-mcp}` — the showcase's tool over its MCP endpoint — then a line of text. Made the same way |
| `says-markdown` | `says-ready` with its reply's text replaced by markdown: a `sh` code block, a table and a link — what the conversation draws of a reply. Made from `says-ready` by hand: only the text differs |

## Re-recording one

```sh
# with a model configured in .env
node tests/live/record-fixture.mjs <name> "<the prompt>"
```

## Two things they cannot be

**They are not strict.** Replay can compare the live request against the
recorded one and fail on any difference, and that would be the better test —
except the assembled prompt carries **today's date** and **absolute paths**,
so a strict fixture stops matching the morning after it was recorded, and on
any checkout at a different path. Divergences are reported instead. Making
strict possible needs the engine's `environment` seam to be settable by a
host; it is filed.

**They are not a model.** A fixture answers the prompt it was recorded
against. Change what the test sends and the recorded response is still what
comes back — which is exactly why they are useful for testing *the host* and
useless for testing *the model*.
