# Showcase

One of everything a runtime application can bring to this Harnix, in one package — the sample
the LIVE suites install (`tests/live/api/apps-runtime.spec.ts`, `tests/live/api/apps-backend.spec.ts`,
`tests/live/ui/apps.spec.ts`, and for its isolated interface `tests/live/api/apps-frames.spec.ts`,
`tests/live/ui/apps-frames.spec.ts`), and the one the store suites sign, list in a mock store and install
by name (`tests/live/api/apps-store.spec.ts`, `tests/live/ui/apps-store.spec.ts`).

| Part | Where | What it shows |
|---|---|---|
| Manifest | `harnix-app.toml` | the id; the capabilities it asks for (`storage_mb`, `fs`, `net`, `env`, `hub`); one `backend-app` component; its `[[method]]`s (with a params schema), `[[topic]]`s and `[[space]]`; `[config]`; its interface module; its isolated interface (`[interface]`, `[[surface]]`); where its agent package goes |
| Backend | `component/` (Rust, `wasm32-wasip2`), `backend` | one resident instance: a board of notes in the key-value store (`notes.add` / `list` / `clear`, published on the `notes` topic), a counter in its own memory (`counter.bump`), a timer that publishes `heartbeat` (`timer.start`), `fetch` (`host.http-request`, to the one host in `net`), `session.start` (creates a session in its space and sends it a prompt, as `module:example.showcase`), `overreach` (a call its `hub` does not list), `state`, and `spin` / `crash` (a timeout recycles the instance; a crash rebuilds it once, a second sets the application aside). `init` subscribes to `harnix.sessions.status` |
| Tools | `component/`, `tools` | `echo`, `count` (key-value storage), `config` (`host.config()`), `read_project` (the session's project, read-only), `slow` (progress, cancellation), `vetoed`, `board` (the backend's state, asked for with `host.call`: a tool's instance cannot see the backend's memory) |
| Event hook | `component/` (`events`) | refuses the tool its configuration names (`blocked_tool`), on every tool call of the session |
| Agent package | `agent/plugin.toml` | a QuickJS `prompt.block` (`scripts/block.js`) and an agent type, `showcase-guide` |
| Skill | `agent/skills/showcase-tour/` | copied into each instance the package goes to, and taken out with it |
| Interface, in the page | `ui/showcase.js` (`[[ui]]`) | a `home.card`, and the showcase's space: the board, which calls the backend's methods and draws what its topics push — over the page's own connection, on the shell's React, with its own strings. Runs only when a publisher this Harnix trusts signed the package |
| Interface, in frames | `ui/frame/` (`[interface]`, `[[surface]]`) | the same, and more, in isolated frames: every surface the shell offers — the space's centre (the board), its left column, a right-column tab, its second panel, a details panel, a home card, a settings section, a modal — drawn by one page on `@harnix/app-sdk` and `@harnix/ui-kit`, through the shell's bridge only. How it runs when it is store-signed or unsigned (see below) |

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

## The interface, in frames

Who signed the package decides where its interface runs: a publisher this Harnix trusts, in the
page (`ui/showcase.js`); anyone else — the store, or nobody — in isolated frames, and
`ui/showcase.js` does not run at all. A trusted package runs in frames too with `isolate = true`
under `[interface]`.

`ui/frame/index.html` is every surface: the shell tells it which one it is when it connects, and
`surfaces.js` draws that one. Each frame is sandboxed (`allow-scripts allow-forms`: an opaque
origin, no storage, no cookies, the page's token out of reach) and served under a policy with
`connect-src 'none'`, so everything goes through the shell's bridge (`harnix-ui-bridge@1`):

| On the board | What it does through the bridge |
|---|---|
| Pin it / Compose… | `app.example.showcase.notes.add`; Compose opens its `compose` modal (`openModal`), which answers the note |
| a note | its second panel (`secondary.open`), with the note as params |
| Count one more / Start the heartbeat | `counter.bump`, `timer.start`; the `heartbeat` topic, here and in the right column's tab |
| Reach too far | `harnix.apps.list`, which its `[interface] hub` does not list: refused, and listed in the system space's Apps tab and in Diagnostics |
| Clear the board | the shell's own confirmation (`confirm`), then `notes.clear` |
| Say hello | a notice under the showcase's name (`notify`) |
| Crash | the page goes away; the shell's frame says so and offers a reload. Three in a row set it aside |

The left column counts the space's notes and sessions (`harnix.sessions`, in its hub) and starts
a session there (`session.create`); the space's entry carries the number of notes (`badge`); the
card's link opens only after the shell asks (`[interface] links`). Theme and language follow the
shell's (`HarnixKit.follow`).

`ui/frame/vendor/` is `@harnix/app-sdk` and `@harnix/ui-kit` as built (classic scripts: a frame's
origin is opaque, and a module script would need CORS). `node tests/app-ui-vendored.mjs --write`
builds them again; the front-end check fails when they are stale.

Under `[apps] dev = true`, an unsigned application's frames reload when a file under its
installed `ui/` changes (`<apps dir>/example.showcase/0.1.0/ui/`, `harnix.apps.list` → `dir`): edit
there, and the page follows.

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

`harnix-app build examples/apps/showcase` does the first half alone: the component, and its
artifact compiled for this engine, in `build/`. Artifacts are compiled for the target's baseline
CPU, so one built here loads on any machine of the same target triple.

## Install, unsigned

A Harnix built with the `apps` feature, and a profile that hosts applications and runs unsigned
ones (`dev = true`, for developing an application):

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

## Sign, publish, install by name

A signed package runs without `dev`. Its publisher signs every file; a store checks that
signature, adds the components it compiled, and countersigns:

```sh
T=target/apps/release/harnix-app
$T keygen publisher.key                       # prints the public key
$T sign showcase.hxapp --key publisher.key
# The store's side:
mkdir -p store/packages
$T countersign showcase.hxapp --key store.key -o store/packages/example.showcase-0.1.0.hxapp
$T index store --name "My store"              # store/index.json
```

A Harnix installs a package countersigned with the store key it carries built in, or one a
publisher it trusts signed in full:

```toml
[apps]
enabled = true
store = "https://store.example.com/index.json"   # or file:///…/store/index.json

[apps.trusted_publishers]
example = ["<the publisher's public key>"]
```

`harnix.apps.catalog` lists what the store has for this Harnix's engine.
`harnix.apps.install {"id": "example.showcase"}` downloads the newest version, checks it against
the sha256 the index lists and against its signatures, and answers what the application asks for
and a `digest`; the same call with `"consent": "<digest>"` installs it. `harnix.apps.update` lists
only what a newer version asks for beyond the installed one, and asks again only then. Settings →
Applications does the same in the page.
