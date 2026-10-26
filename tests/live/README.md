# LIVE tests

Playwright Test against the real `harnix` binary: `api/` drives the backend
through its own protocol, `ui/` drives the interface in chromium.

```sh
pnpm live                       # both, model calls replayed
pnpm live --project api --live  # the API suites against the model in .env
```

How to run, write, record and read the coverage table: [docs/testing.md](../../docs/testing.md).
