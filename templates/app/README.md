# An application on Harnix

```sh
cp -R harnix/templates/app <your-app> && cd <your-app> && git init
git submodule add https://github.com/openatta/harnix.git harnix
git submodule add https://github.com/openatta/agix.git agix
git -C agix checkout "$(git -C harnix ls-tree HEAD agix | awk '{print $3}')"   # Harnix's pin
node harnix/scripts/sync-app-deps.mjs .       # writes the [workspace.dependencies] block
node harnix/scripts/check-pins.mjs .
scripts/build.sh
./target/release/myapp --profile profiles/myapp.toml
```

Do not initialise `harnix/agix`: here `agix/` sits beside `harnix/`, and that is
the one Harnix's crates are built against.

Rename `myapp` (in `app/Cargo.toml`, `app/src/main.rs`, `profiles/`,
`scripts/build.sh`), then add your first module: see
`harnix/docs/harnix-sdk.md`.
