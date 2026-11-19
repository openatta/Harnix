/**
 * What a product says about itself (concept §5.10): `[brand]` — its name, its icon, and
 * the id its own directory is named after — and `[shell] spaces`, which of the built-in spaces it
 * keeps.
 */

import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { SPACE_MISMATCH } from "@harnix/core/protocol";
import { type Backend, connect, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { startTestApp } from "./testapp";

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
      profile: `\n[brand]\nname = "Acme Desk"\nproduct_id = "acme"\nproduct_name = "AcmeDesk"\nicon = "acme.svg"\n`,
    }),
  );
  try {
    const api = await connect(backend);
    const { shell } = await api.call("harnix.hello");
    expect(shell.brand).toEqual({ name: "Acme Desk", icon: "/brand/icon.svg", product_id: "acme", project_dir: ".acme" });
    expect(shell.state_dir).toBe(backend.state);
    expect(backend.state.endsWith("/.acme")).toBe(true);
    // Nothing of the program's own lands among the projects any more.
    expect(existsSync(join(backend.scratch, "projects", ".harnix"))).toBe(false);
    const served = await fetch(`${backend.url}brand/icon.svg`);
    expect(served.headers.get("content-type")).toBe("image/svg+xml");
    expect(await served.text()).toBe(ICON);
    api.close();
  } finally {
    await backend.dispose();
  }
});

test("without a brand the state is ~/.harnix, and a product id that is not a name is refused", BRAND, async () => {
  const backend = await startHarnix(launchFor("replay"));
  try {
    const api = await connect(backend);
    const { shell } = await api.call("harnix.hello");
    expect(shell.brand.product_id).toBe("harnix");
    expect(shell.brand.project_dir).toBe(".harnix");
    expect(shell.brand.name ?? null).toBe(null);
    expect(backend.state.endsWith("/.harnix")).toBe(true);
    expect((await fetch(`${backend.url}brand/icon.svg`)).headers.get("content-type")).not.toBe("image/svg+xml");
    api.close();
  } finally {
    await backend.dispose();
  }
  await expect(startHarnix(launchFor("replay", { profile: '\n[brand]\nproduct_id = "../etc"\n' }))).rejects.toThrow(/product_id/);
});

test("a branded product keeps its own directory in each project: the engine's project tier and a task's runs in .<product_id>", BRAND, async () => {
  const backend = await startTestApp({ profile: '\n[brand]\nname = "Acme"\nproduct_id = "acme"\n' });
  const project = realpathSync(mkdtempSync(join(tmpdir(), "harnix-brand-")));
  try {
    const api = await connect(backend);
    expect((await api.call("harnix.hello")).shell.brand.project_dir).toBe(".acme");

    // The engine's project tier: a file put in `.acme/` is read, one in `.harnix/` is not, and the
    // engine writes the tier there.
    const at = { project_root: project };
    const field = async (key: string) => (await api.call("settings.describe", at)).fields.find((f: { key: string }) => f.key === key);
    mkdirSync(join(project, ".harnix"));
    writeFileSync(join(project, ".harnix", "settings.json"), JSON.stringify({ permission_rules: [{ tool: "Bash(curl:*)", action: "deny" }] }));
    mkdirSync(join(project, ".acme"));
    writeFileSync(join(project, ".acme", "settings.json"), JSON.stringify({ permission_rules: [{ tool: "Bash(wget:*)", action: "deny" }] }));
    const read = await field("permission_rules");
    expect(read.source).toBe("project");
    expect(JSON.stringify(read.tiers.project)).toContain("wget");
    expect(JSON.stringify(read.tiers.project)).not.toContain("curl");
    expect((await api.call("settings.describe", at)).paths.project).toBe(join(project, ".acme", "settings.json"));
    rmSync(join(project, ".harnix"), { recursive: true });
    await api.call("settings.write", { tier: "project", changes: { permission_rules: [{ tool: "Bash(rm:*)", action: "deny" }] }, ...at });
    expect(readFileSync(join(project, ".acme", "settings.json"), "utf8")).toContain("Bash(rm:*)");

    // A task's process directory, and the one line the flow adds to the directory's `.gitignore`.
    const { run } = await api.call("flow.start", { flow: "demo-task", subject: "repo", project_root: project });
    const sub = await api.call("harnix.topics.subscribe", { topic: "flow.run", params: { run } });
    if (sub.value?.record?.status !== "finished") {
      await api.waitFor(
        (f) => f.method === "harnix.topic" && f.params.topic === "flow.run" && f.params.params?.run === run && f.params.value?.record?.status === "finished",
        { timeout: 60_000, describe: `${run} finishes` },
      );
    }
    const task = JSON.parse(readFileSync(join(project, ".acme", "runs", run, "task.json"), "utf8"));
    expect(task).toMatchObject({ run, flow: "demo-task", status: "finished" });
    const ignored = readFileSync(join(project, ".acme", ".gitignore"), "utf8").split("\n");
    expect(ignored).toContain("runs/");
    // The engine's local data is in the product's directory too, under its `state/`.
    expect(ignored).toContain("state/");
    // Nothing of Harnix's own name in the project.
    expect(readdirSync(project).sort()).toEqual([".acme", "flows", "notes.txt"]);
    api.close();
  } finally {
    await backend.dispose();
    rmSync(project, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
  }
});

test("a brand whose name the engine cannot go by is refused at start, saying which key to set", BRAND, async () => {
  await expect(startHarnix(launchFor("replay", { profile: '\n[brand]\nname = "Acme Desk"\nproduct_id = "acme"\n' }))).rejects.toThrow(/product_name/);
  await expect(startHarnix(launchFor("replay", { profile: '\n[brand]\nproduct_id = "github"\n' }))).rejects.toThrow(/project_dir/);
});

test("a product switches off the extensions whose spaces it does not want, and their methods go too", SHELL, async () => {
  const backend = await startHarnix(launchFor("replay", { profile: '\n[extensions]\noff = ["flow"]\n' }));
  try {
    const api = await connect(backend);
    const { extensions } = await api.call("harnix.hello");
    const enabled = extensions.filter((e: any) => e.enabled).map((e: any) => e.id);
    expect(enabled).toContain("harnix-app");
    expect(enabled).not.toContain("flow");
    const reachable = (await api.call("harnix.reachable")).methods as string[];
    expect(reachable.some((m) => m.startsWith("flow."))).toBe(false);
    expect((await api.call("harnix.spaces.list")).spaces.map((s: any) => s.id)).not.toContain("flow");
    const created = await api.call("harnix.session.create", { space: "sessions" });
    const refused = await api.refused("harnix.session.attach", { instance: created.instance, session_id: created.session_id, space: "flow" });
    expect(refused?.code).toBe(SPACE_MISMATCH);
    api.close();
  } finally {
    await backend.dispose();
  }
  await expect(startHarnix(launchFor("replay", { profile: '\n[shell]\nspaces = ["sessions"]\n' }))).rejects.toThrow(/spaces/);
});
