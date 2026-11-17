# An application on Harnix

```sh
cp -R harnix/templates/app <your-app> && cd <your-app> && git init
git submodule add https://github.com/openatta/harnix.git harnix
git submodule update --init --recursive        # Harnix brings everything it is built on
node harnix/scripts/sync-app-deps.mjs .       # writes the [workspace.dependencies] block
node harnix/scripts/check-pins.mjs .
scripts/build.sh
./target/release/myapp --profile profiles/myapp.toml
```

`harnix/` is the only submodule the application carries. A problem or a missing capability
is a Harnix issue, whatever layer it turns out to live in.

Rename `myapp` (in `app/Cargo.toml`, `app/src/main.rs`, `profiles/`,
`scripts/build.sh`), then add your first module: see
`harnix/docs/harnix-ext.md`.

The profile's `[brand]` is your product's names: `product_id` names its state directory
(`~/.myapp`) and its directory in each project (`.myapp/`, where the engine's project settings and
your extensions' project data go), and `name` is what the engine calls itself. Both can be set apart
(`project_dir`, `product_name`); one the engine cannot take stops the start, naming the key
(`harnix/docs/harnix-ext.md` §18).
