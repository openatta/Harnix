# hello-app

An application on Harnix, laid out and built the way one is (docs/harnix-sdk.md §3): `harnix/` and
`agix/` beside it (here, links into this checkout), its own Cargo workspace, its own interface
modules.

| space | interface (`ui/`) | backend (`app/`) | Agent |
|---|---|---|---|
| `greeter` | a right-column tab and a HOME card | the `greeter` module: `app.greeter.greet`, the `app.greeter.greeted` topic, the space's default instance | an in-process instance built from the module's template, the first time the space needs one |
| `notes` | a space | — | the `system` instance |

HOME and the session space come with Harnix.

```sh
scripts/build.sh            # checks, the interface with ui/ compiled in, then the binary embedding it
target/release/hello-app --profile profiles/hello.toml
```

The build sets three variables (design §11.4): `HARNIX_UI_MODULES` names `ui/src/index.tsx` for the
shell build, `HARNIX_UI_OUT` puts that build in `target/ui`, and `HARNIX_UI_DIST` makes the binary
embed it instead of Harnix's own interface.

`tests/live/ui/spaces.spec.ts` drives it.
