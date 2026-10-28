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
| `runs-a-while` | A `Bash` call that waits four seconds (in `perl`: the engine refuses a `sleep` of 2 s or more): a turn that is still running when something is steered into it (ipdrun-13). Made the same way |

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
