# Showcase

One of everything a runtime application can bring to this Harnix, in one package — the sample
the LIVE suites install (`tests/live/api/apps-runtime.spec.ts`, `tests/live/api/apps-backend.spec.ts`,
`tests/live/ui/apps.spec.ts`).

| Part | Where | What it shows |
|---|---|---|
| Manifest | `harnix-app.toml` | the id; the capabilities it asks for (`storage_mb`, `fs`, `net`, `env`, `hub`); one `backend-app` component; its `[[method]]`s (with a params schema), `[[topic]]`s and `[[space]]`; `[config]`; its interface module; where its agent package goes |
| Backend | `component/` (Rust, `wasm32-wasip2`), `backend` | one resident instance: a board of notes in the key-value store (`notes.add` / `list` / `clear`, published on the `notes` topic), a counter in its own memory (`counter.bump`), a timer that publishes `heartbeat` (`timer.start`), `fetch` (`host.http-request`, to the one host in `net`), `session.start` (creates a session in its space and sends it a prompt, as `module:example.showcase`), `overreach` (a call its `hub` does not list), `state`, and `spin` / `crash` (a timeout recycles the instance; a crash rebuilds it once, a second sets the application aside). `init` subscribes to `harnix.sessions.status` |
| Tools | `component/`, `tools` | `echo`, `count` (key-value storage), `config` (`host.config()`), `read_project` (the session's project, read-only), `slow` (progress, cancellation), `vetoed`, `board` (the backend's state, asked for with `host.call`: a tool's instance cannot see the backend's memory) |
| Event hook | `component/` (`events`) | refuses the tool its configuration names (`blocked_tool`), on every tool call of the session |
| Agent package | `agent/plugin.toml` | a QuickJS `prompt.block` (`scripts/block.js`) and an agent type, `showcase-guide` |
| Skill | `agent/skills/showcase-tour/` | copied into each instance the package goes to, and taken out with it |
| Interface | `ui/showcase.js` | a `home.card`, and the showcase's space: the board, which calls the backend's methods and draws what its topics push — over the page's own connection, on the shell's React, with its own strings |

A session sees the tools as `app__example_showcase__<tool>`; each one asks before it runs. A
daemon this Harnix spawned sees them as `mcp__app-example_showcase__<tool>`, over the
application's MCP endpoint: it runs no WebAssembly of its own, and the call runs in this process.

## The backend's methods

`app.example.showcase.<method>`, in the authorization table like any method: the page calls them
under its token (they are declared for `device`), and the application itself always may (its
`board` tool asks `state`). `state` is also open to modules. The params of `notes.add`,
`timer.start`, `fetch` and `session.start` are checked against the schema in the manifest before
the backend sees them. A client subscribes to `app.example.showcase.notes` and
`app.example.showcase.heartbeat` with `harnix.subscribe`.

What the backend does as itself is admitted against its `hub`: `session.create`, `harnix.send`, and
the topic `harnix.sessions.status`. Anything else — `overreach` tries `harnix.apps.uninstall` — is
refused, on the record (`harnix.audit`, subject `module:example.showcase`), and listed in the
system space's Apps tab with the application's VM health.

## Build and pack

You need the `wasm32-wasip2` target (`rustup target add wasm32-wasip2`) and the packing tool,
which links the engine's compiler and so is never part of the product:

```sh
cargo build --release -p harnix-apps --features compile --bin harnix-app --target-dir target/apps
target/apps/release/harnix-app pack examples/apps/showcase --build -o showcase.hxapp
```

`--build` runs `cargo build --release --target wasm32-wasip2` in `component/` (or give an already
built component with `--wasm <file>`); `pack` compiles it for this machine's engine and zips the
manifest, `ui/`, `agent/` and both forms of the component. It prints the package's path and the
engine fingerprint: the package installs on a Harnix whose engine reports the same one
(`harnix.apps.list` → `runtime.fingerprint`; `harnix.hello` → `apps_engine`).

## Install

A Harnix built with the `apps` feature, and a profile that hosts applications and trusts unsigned
ones (there are no signatures before the store):

```sh
cargo build --release -p harnix-cli --features apps --target-dir target/apps
cat > /tmp/apps.toml <<'EOF'
[apps]
enabled = true
dev = true

# Optional: set over the application's own [config].
[apps.config."example.showcase"]
greeting = "hello from my profile"
EOF
target/apps/release/harnix --profile /tmp/apps.toml
```

Then, from any client: `harnix.apps.installFile {"path": "/abs/path/showcase.hxapp"}`. New
sessions have the tools, the prompt block and the agent type; the home screen has the card, and
the left column the showcase's space. `fetch` reaches `127.0.0.1` only (any port): start something
there to try it. `state` reports `secret_set: true` when `SHOWCASE_TOKEN` is set for the process.
`harnix.apps.disable` / `enable {"id": "example.showcase"}` switch it, `harnix.apps.uninstall`
takes it out, its stored counts and notes with it. After a second crash the application is set
aside (`harnix.apps.list` → `runtime.state: "faulted"`); `harnix.apps.enable` starts it again.
