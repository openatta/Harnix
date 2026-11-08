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
| `leaves-a-process` | A `Bash` call that leaves a `tail -f /dev/null` in the background and prints its pid (ipdrun-07). Made from `calls-a-tool` by hand: only the tool input and the text differ |
| `calls-a-native-tool` | A call to `ProbeFixed`, the tool `harnix-testapp`'s native scene contributes (ipdrun-10). Made the same way |
| `flow-analyze` | The flow module's `analyze` station (xops-01): one call to `flow_submit` with `{findings: [one low finding]}`, then a line of text. Made from `calls-a-native-tool` by hand; `harnix-testapp` names a session task's recording `flow-<station>` |
| `flow-report` | The same for the `report` station's `summary` task: `flow_submit` with `{summary}` |
| `flow-propose` | One turn of four calls: `TodoWrite` (which `harnix-testapp` guards as a write, so the flow refuses it), `flow_propose {kind: rework, to: propose}`, `flow_submit {done: true}`, a line of text. Made from `calls-a-native-tool` by hand |
| `flow-shared` | Two turns in one session (`per_run`): each calls `flow_submit` with its own `step` and says a line; between them a filler `[]` answers the memory-extraction call the engine makes after every finished turn, which otherwise takes the next turn's recorded call. Made the same way |
| `flow-local`, `flow-remote` | xops-02: a station's session on a template instance (in process / spawned) hands in by structured output — one call to `StructuredOutput` with `{findings: [one low finding]}`, then a line of text. Made the same way, with `StructuredOutput` as the tool |
| `flow-guarded`, `flow-bypassed` | xops-02: `TodoWrite` (refused by the session's `deny` permission rule — for `flow-bypassed` on an instance set to `bypassPermissions`), then `StructuredOutput {done: true}`, then a line of text |
| `flow-author-unfocused` | xops-02: a `flow-author` conversation calls `flow_update` with no flow selected (and is told to ask for a selection) |
| `flow-author-edit` | xops-02: three `flow-author` turns — `flow_schema`, `flow_validate`, `flow_create` · `flow_show`, `flow_update`, `flow_dry_run` · `flow_delete` — with a filler `[]` between turns, as in `flow-shared` |
| `flow-ui-edit` | xops-02 §7, the flow space in the browser: one `flow-author` turn — `flow_show`, then `flow_update {flow: example-review, yaml}` with the empty application's `example-review` retitled "Example: reviewed in conversation", then a line of text — and a filler `[]` for the memory extraction after it. Made from `flow-author-unfocused` by hand |
| `runs-a-while` | A `Bash` call that waits four seconds (in `perl`: the engine refuses a `sleep` of 2 s or more): a turn that is still running when something is steered into it (ipdrun-13). Made the same way |
| `many-turns` | `says-ready`'s one call sixty times over: a session long enough to compact (eight turns), whichever call comes next — a turn, a memory extraction, a compaction's summary — answered the same. Made by `python3` from `says-ready`, sequence numbers renumbered |
| `streams-output` | A `Monitor` call (a `code`-scene tool) whose command prints a line every quarter second for ten seconds: a running task whose output grows. Made from `leaves-a-process` by hand: the tool's name and input differ |
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
