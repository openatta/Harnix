# Package fixtures

Two package sources, each packed beside it the way the engine's installer takes one:

| Package | What it is for |
|---|---|
| `demo-rows/` → `demo-rows.zip` | A `tool.row` written against contribution API **1** (a DOM function). The shell does not load it: its entry is listed as "这一条不生效" with the reason, and the rest of the package is untouched (design §10.5, M8-2). |
| `demo-rows-v2/` → `demo-rows-v2.zip` | The same row against contribution API **2**: a precompiled ES module that imports `react/jsx-runtime` and `@harnix/react` from the page's import map, brings its own language pack, and wins the claim for `Bash` over the built-in row (M8-1). |

`tests/package-e2e.mjs` installs both through the real engine and checks the whole path: the file goes
up the bulk channel, the engine installs and discloses it, Harnix reads the one section the engine
ignores and serves the directory same-origin, the browser imports the v2 module, and the CSP admits
the import map by its hash and nothing else.

Re-pack after editing:

```sh
cd tests/fixtures/packages/demo-rows && zip -qrX ../demo-rows.zip .
cd ../demo-rows-v2 && zip -qrX ../demo-rows-v2.zip .
```
