// `pnpm live` — Playwright Test, with this repository's own flags in front.
//
//   --live                 the model in `.env` instead of the recordings
//   --no-model             nothing model-shaped
//   --topology <t>         what the API suites speak (single_duplex by default)
//
// Everything else goes to `playwright test` as is.

import { spawnSync } from "node:child_process";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";

const argv = process.argv.slice(2);
const env = { ...process.env };
const rest = [];
for (let i = 0; i < argv.length; i += 1) {
  const a = argv[i];
  if (a === "--live") env.HARNIX_MODEL = "live";
  else if (a === "--no-model") env.HARNIX_MODEL = "none";
  else if (a === "--topology") env.HARNIX_TOPOLOGY = argv[++i];
  else rest.push(a);
}
const result = spawnSync("pnpm", ["exec", "playwright", "test", ...rest], {
  cwd: dirname(fileURLToPath(import.meta.url)),
  env,
  stdio: "inherit",
});
process.exit(result.status ?? 1);
