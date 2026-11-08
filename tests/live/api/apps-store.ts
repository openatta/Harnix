/**
 * What the store suites share: a mock store, and a Harnix that trusts it.
 *
 * The mock store is a directory (`file://…/index.json`) made here, from the showcase as
 * `apps-host.ts` packs it: signed by the mock publisher, countersigned by the mock store, listed by
 * `harnix-app index` — the same tools a real store and publisher use. Its keys are committed in
 * `tests/live/fixtures/store/` (`*.key` secrets, `*.pub` public keys); they are test keys and
 * nothing else. A Harnix carries the store's own key built in and cannot be told another one by a
 * profile: only `harnix-testapp`, assembled with the mock store's key (`AppBuilder::store_keys`,
 * hidden), trusts this store. It is built with `apps` into `target/apps`, beside the apps binary.
 *
 * Two versions of the showcase are listed: 0.1.0, as it is, and 0.2.0, asking for more — an
 * environment variable, a host, a larger store — so an update has something to disclose.
 */

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";

import { type Backend, type LaunchOptions, type ModelMode, ROOT, launchFor, startHarnix } from "@harnix/testkit";

import { type Entries, SHOWCASE_ID, appsTools, readZip, showcasePackage, writeZip } from "./apps-host";

export const STORE_FIXTURE = join(ROOT, "tests", "live", "fixtures", "store");
const key = (name: string) => join(STORE_FIXTURE, name);
export const MOCK_PUBLISHER = readFileSync(key("mock-publisher.pub"), "utf8").trim();

let testapp: string | null = null;

/** `harnix-testapp` with `apps`: the one binary that trusts the mock store. */
export function appsTestApp(): string {
  if (testapp) return testapp;
  const { bin } = appsTools();
  testapp = join(dirname(bin), "harnix-testapp");
  if (!process.env.HARNIX_APPS_BIN || !existsSync(testapp)) {
    execFileSync(
      "cargo",
      ["build", "--release", "--target-dir", join(ROOT, "target", "apps"), "-p", "harnix-testapp", "--features", "apps"],
      { cwd: ROOT, stdio: ["ignore", "ignore", "inherit"] },
    );
  }
  return testapp;
}

export const sha256 = (path: string) => createHash("sha256").update(readFileSync(path)).digest("hex");

const tool = (args: string[]) => execFileSync(appsTools().pack, args, { encoding: "utf8" });

export interface MockStore {
  dir: string;
  /** `[apps] store`. */
  url: string;
  /** The packages as listed: by version. */
  packages: Record<string, string>;
  /** The publisher-signed 0.1.0, which no store countersigned. */
  publisherOnly: string;
  /** The index as made; `write` replaces what is served. */
  index: any;
  write(index: any): void;
  /** A package derived from a listed one, put under `packages/`: its path. */
  derive(version: string, name: string, edit: (entries: Entries) => void): string;
}

/** The 0.2.0 manifest: the same application, asking for more. */
function grown(manifest: string): string {
  return manifest
    .replace('version = "0.1.0"', 'version = "0.2.0"')
    .replace("storage_mb = 1", "storage_mb = 2")
    .replace('env = ["SHOWCASE_TOKEN"]', 'env = ["SHOWCASE_TOKEN", "SHOWCASE_EXTRA"]')
    .replace('net = ["127.0.0.1"]', 'net = ["127.0.0.1", "api.example.com"]');
}

/** A fresh mock store in a temporary directory. */
export function mockStore(): MockStore {
  const dir = mkdtempSync(join(tmpdir(), "hx-store-"));
  const staging = join(dir, "staging");
  mkdirSync(join(dir, "packages"), { recursive: true });
  mkdirSync(staging, { recursive: true });
  const unsigned = readZip(showcasePackage().path);
  const versions: Record<string, Entries> = {
    "0.1.0": unsigned,
    "0.2.0": new Map([...unsigned].map(([n, b]) => [n, n === "harnix-app.toml" ? Buffer.from(grown(b.toString())) : b])),
  };
  const packages: Record<string, string> = {};
  for (const [version, entries] of Object.entries(versions)) {
    const plain = join(staging, `${version}.hxapp`);
    writeZip(plain, entries);
    const signed = join(staging, `${version}.signed.hxapp`);
    tool(["sign", plain, "--key", key("mock-publisher.key"), "-o", signed]);
    const listed = join(dir, "packages", `${SHOWCASE_ID}-${version}.hxapp`);
    tool(["countersign", signed, "--key", key("mock-store.key"), "-o", listed]);
    packages[version] = listed;
  }
  tool(["index", dir, "--name", "Mock store"]);
  const indexPath = join(dir, "index.json");
  const index = JSON.parse(readFileSync(indexPath, "utf8"));
  return {
    dir,
    url: pathToFileURL(indexPath).href,
    packages,
    publisherOnly: join(staging, "0.1.0.signed.hxapp"),
    index,
    write(next: any) {
      writeFileSync(indexPath, JSON.stringify(next, null, 2));
    },
    derive(version: string, name: string, edit: (entries: Entries) => void): string {
      const entries = readZip(packages[version]);
      edit(entries);
      const path = join(dir, "packages", `${name}.hxapp`);
      writeZip(path, entries);
      return path;
    },
  };
}

/** The index with `version`'s artifacts replaced. */
export function withArtifacts(index: any, version: string, artifacts: Record<string, unknown>): any {
  const next = structuredClone(index);
  const v = next.apps[0].versions.find((x: any) => x.version === version);
  v.artifacts = artifacts;
  return next;
}

/** A backend that hosts applications from `store`, with `dev` off unless said. `bin`: the
 *  apps testapp (trusts the mock store) unless another binary is given. */
export function startStore(
  store: MockStore,
  options: { bin?: string; dev?: boolean; profile?: string; mode?: ModelMode; topologies?: LaunchOptions["topologies"] } = {},
): Promise<Backend> {
  return startHarnix(
    launchFor(options.mode ?? "replay", {
      bin: options.bin ?? appsTestApp(),
      topologies: options.topologies,
      profile: `
[apps]
enabled = true
dev = ${options.dev ?? false}
store = "${store.url}"
${options.profile ?? ""}`,
    }),
  );
}
