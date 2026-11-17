// A project's trust, through the projects space's own methods (docs/harnix-app.md §3): a device
// holds no capability, so `harnix-app` reads and writes the engine's `project.*` for it, and only
// for a project the device can see — one added as a project, or one a listed session runs in.
//
// Each test makes its own project: a scratch directory whose `.agix/settings.json` asks for what
// an untrusted project may not have (a looser permission mode, an MCP server of its own). No
// session runs in it, so trusting it starts nothing.

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { assert, defineSuite, scratchDir } from "@harnix/testkit";

/** A project directory with settings that wait on trust, added as a project. */
async function project(client: { call(method: string, params?: unknown): Promise<any> }, name: string) {
  const dir = scratchDir(`harnix-trust-${name}-`);
  mkdirSync(join(dir, ".agix"), { recursive: true });
  writeFileSync(
    join(dir, ".agix", "settings.json"),
    JSON.stringify({ permission_mode: "bypassPermissions", mcp_servers: { demo: { command: "true" } } }, null, 2),
  );
  const made = await client.call("harnix-app.workspaces.create", { path: dir, title: name });
  return { dir, id: made.workspace.id as string };
}

defineSuite("project trust", { feature: "projects" }, {
  tests: {
    "a project is untrusted until trusted, and what it held back is in force once it is": async ({ client }) => {
      const { dir, id } = await project(client, "status");
      try {
        const before = await client.call("harnix-app.projects.status", { project_root: dir });
        assert(before.trust === "untrusted", `starts ${before.trust}`);
        const keys: string[] = before.items.map((i: any) => i.key);
        assert(keys.includes("permission_mode") && keys.includes("mcp_servers.demo"), `waits on ${keys}`);
        assert(before.categories.includes("permissions") && before.categories.includes("mcp"), `kinds ${before.categories}`);
        assert(before.withheld.length === before.items.length, "an untrusted project's items are not all withheld");
        const demo = before.items.find((i: any) => i.key === "mcp_servers.demo");
        assert(demo.category === "mcp" && demo.name === "demo", JSON.stringify(demo));

        const trusted = await client.call("harnix-app.projects.trust", { project_root: dir, trusted: true });
        assert(trusted.trust === "trusted" && trusted.trust_source === "host", JSON.stringify(trusted));
        assert(/settings\.local\.json$/.test(trusted.written_to), `written to ${trusted.written_to}`);
        assert(trusted.withheld.length === 0, "a trusted project still withholds");

        // Read again, it is what was written; and the topic says so to every page.
        const after = await client.call("harnix-app.projects.status", { project_root: dir });
        assert(after.trust === "trusted" && after.items.length === before.items.length, JSON.stringify(after));
        const topic = (await client.call("harnix.topics.subscribe", { topic: "harnix-app.trust", params: {} })).value;
        assert(topic?.projects?.[after.project_root]?.trust === "trusted", JSON.stringify(topic));

        // No longer: the entry for this directory alone goes, and it is untrusted again.
        const revoked = await client.call("harnix-app.projects.trust", { project_root: dir, trusted: false });
        assert(revoked.trust === "untrusted" && revoked.withheld.length === before.items.length, JSON.stringify(revoked));
      } finally {
        await client.call("harnix-app.projects.trust", { project_root: dir, trusted: false }).catch(() => undefined);
        await client.call("harnix-app.workspaces.remove", { id });
      }
    },

    "a session's project status is the project it runs in, and one with no project has nothing withheld": async ({ client }) => {
      const { dir, id } = await project(client, "session");
      const made = await client.call("harnix.session.create", { project_root: dir });
      const loose = await client.call("harnix.session.create", { project_root: null });
      try {
        const status = await client.call("harnix-app.sessions.projectStatus", { session_id: made.session_id, space: "sessions" });
        assert(status.session_id === made.session_id && status.trust === "untrusted", JSON.stringify(status));
        assert(status.withheld.some((i: any) => i.key === "mcp_servers.demo"), JSON.stringify(status.withheld));
        const none = await client.call("harnix-app.sessions.projectStatus", { session_id: loose.session_id, space: "sessions" });
        assert(none.trust === "none" && none.withheld.length === 0, JSON.stringify(none));
      } finally {
        for (const session of [made, loose]) {
          await client.call("harnix.session.delete", { session_id: session.session_id }).catch(() => undefined);
        }
        await client.call("harnix-app.workspaces.remove", { id });
      }
    },

    "a directory that is not a project here is neither read nor trusted": async ({ client }) => {
      const stranger = scratchDir("harnix-trust-stranger-");
      for (const [method, params] of [
        ["harnix-app.projects.status", { project_root: stranger }],
        ["harnix-app.projects.trust", { project_root: stranger, trusted: true }],
      ] as const) {
        const e = await client.refused(method, params);
        assert(e && /not a project here/.test(e.message), `${method}: ${e?.message}`);
      }
      // Nor does a device reach the engine's own methods: it holds no capability.
      for (const method of ["engine.project.status", "engine.project.trust"]) {
        const e = await client.refused(method, { project_root: stranger, trusted: true });
        assert(e, `${method} was let through to a device`);
      }
    },
  },
});
