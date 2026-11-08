# Showcase

One of everything a runtime application can bring to this Harnix, in one package — the sample
the LIVE suites install (`tests/live/api/apps-runtime.spec.ts`, `tests/live/ui/apps.spec.ts`), and
the one the store suites sign, list in a mock store and install by name
(`tests/live/api/apps-store.spec.ts`, `tests/live/ui/apps-store.spec.ts`).

| Part | Where | What it shows |
|---|---|---|
| Manifest | `harnix-app.toml` | the id, the capabilities it asks for (`storage_mb`, `fs = ["${project}"]`), one component, `[config]`, an interface module, where its agent package goes |
| Tools | `component/` (Rust, `wasm32-wasip2`) | `echo`, `count` (key-value storage), `config` (`host.config()`), `read_project` (the session's project, read-only), `slow` (progress, cancellation), `vetoed` |
| Event hook | `component/` (`events`) | refuses the tool its configuration names (`blocked_tool`), on every tool call of the session |
| Agent package | `agent/plugin.toml` | a QuickJS `prompt.block` (`scripts/block.js`) and an agent type, `showcase-guide` |
| Skill | `agent/skills/showcase-tour/` | copied into each instance the package goes to, and taken out with it |
| Interface | `ui/showcase.js` | a `home.card`, on the shell's React, with its own strings |

A session sees the tools as `app__example_showcase__<tool>`; each one asks before it runs. A
daemon this Harnix spawned sees them as `mcp__app-example_showcase__<tool>`, over the
application's MCP endpoint: it runs no WebAssembly of its own, and the call runs in this process.

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
sessions have the tools, the prompt block and the agent type; the home screen has the card.
`harnix.apps.disable` / `enable {"id": "example.showcase"}` switch it, `harnix.apps.uninstall`
takes it out, its stored counts with it.

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
