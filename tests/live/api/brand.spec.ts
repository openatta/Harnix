/**
 * What a product says about itself (design §10.5, M8-0, M8-4): `[brand]` — its name, its icon, and
 * the id its own directory is named after — and `[shell] spaces`, which of the built-in spaces it
 * keeps. Plus `harnix.process`, the system space's strip: versions, uptime, memory, and who is
 * connected.
 */

import { existsSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { SPACE_MISMATCH } from "@harnix/core/protocol";
import { type Backend, connect, expect, launchFor, startHarnix, test } from "@harnix/testkit";

const BRAND = { tag: ["@feature:brand", "@layer:api"] };
const SHELL = { tag: ["@feature:shell", "@layer:api"] };

test.describe.configure({ mode: "serial" });

const ICON = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="8" cy="8" r="7"/></svg>';

test("a brand names the product, serves its icon, and keeps its state in ~/.<product_id>", BRAND, async () => {
  let icon = "";
  const backend: Backend = await startHarnix(
    launchFor("replay", {
      seed: (scratch) => {
        icon = join(scratch, "acme.svg");
        writeFileSync(icon, ICON);
      },
      profile: `\n[brand]\nname = "Acme Desk"\nproduct_id = "acme"\nicon = "acme.svg"\n`,
    }),
  );
  try {
    const api = await connect(backend);
    const { shell } = await api.call("harnix.hello");
    expect(shell.brand).toEqual({ name: "Acme Desk", icon: "/brand/icon.svg", product_id: "acme" });
    expect(shell.state_dir).toBe(backend.state);
    expect(backend.state.endsWith("/.acme")).toBe(true);
    // Nothing of the program's own lands among the projects any more.
    expect(existsSync(join(backend.scratch, "projects", ".harnix"))).toBe(false);
    const served = await fetch(`${backend.url}brand/icon.svg`);
    expect(served.headers.get("content-type")).toBe("image/svg+xml");
    expect(await served.text()).toBe(ICON);
    api.close();
  } finally {
    backend.stop();
  }
});

test("without a brand the state is ~/.harnix, and a product id that is not a name is refused", BRAND, async () => {
  const backend = await startHarnix(launchFor("replay"));
  try {
    const api = await connect(backend);
    const { shell } = await api.call("harnix.hello");
    expect(shell.brand.product_id).toBe("harnix");
    expect(shell.brand.name ?? null).toBe(null);
    expect(backend.state.endsWith("/.harnix")).toBe(true);
    expect((await fetch(`${backend.url}brand/icon.svg`)).headers.get("content-type")).not.toBe("image/svg+xml");
    api.close();
  } finally {
    backend.stop();
  }
  await expect(startHarnix(launchFor("replay", { profile: '\n[brand]\nproduct_id = "../etc"\n' }))).rejects.toThrow(/product_id/);
});

test("a product keeps the built-in spaces it names; without the system space its methods go too", SHELL, async () => {
  const backend = await startHarnix(launchFor("replay", { profile: '\n[shell]\nspaces = ["sessions"]\n' }));
  try {
    const api = await connect(backend);
    const { shell } = await api.call("harnix.hello");
    expect(shell.spaces).toEqual(["sessions"]);
    const reachable = (await api.call("harnix.reachable")).methods as string[];
    expect(reachable.some((m) => m.startsWith("harnix.system."))).toBe(false);
    const created = await api.call("session.create", { space: "sessions" });
    const refused = await api.refused("harnix.attach", { instance: created.instance, session_id: created.session_id, space: "system" });
    expect(refused?.code).toBe(SPACE_MISMATCH);
    api.close();
  } finally {
    backend.stop();
  }
  await expect(startHarnix(launchFor("replay", { profile: '\n[shell]\nspaces = ["home"]\n' }))).rejects.toThrow(/home/);
});

test("harnix.process: versions, uptime, peak memory, and every connected user", SHELL, async () => {
  const backend = await startHarnix(launchFor("replay"));
  try {
    const one = await connect(backend);
    const two = await connect(backend);
    const process = await one.call("harnix.process");
    expect(process.harnix).toMatch(/^0\.4\.\d+-\d{10}$/);
    expect(typeof process.agix).toBe("string");
    expect(process.uptime_secs).toBeGreaterThanOrEqual(0);
    expect(process.connections.length).toBeGreaterThanOrEqual(2);
    for (const c of process.connections) expect(c).toMatchObject({ device: expect.any(String), topology: expect.any(String), since_ms: expect.any(Number) });
    two.close();
    await expect.poll(async () => (await one.call("harnix.process")).connections.length).toBe(process.connections.length - 1);
    one.close();
  } finally {
    backend.stop();
  }
});
