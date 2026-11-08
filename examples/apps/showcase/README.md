# Showcase

One of everything a runtime application can bring to this Harnix, in one package — the sample
the LIVE suites install (`tests/live/api/apps-runtime.spec.ts`, `tests/live/ui/apps.spec.ts`).

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
sessions have the tools, the prompt block and the agent type; the home screen has the card.
`harnix.apps.disable` / `enable {"id": "example.showcase"}` switch it, `harnix.apps.uninstall`
takes it out, its stored counts with it.
