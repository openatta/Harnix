// MCP servers from the settings page: added, changed, switched off and on, reconnected, removed,
// in the tier chosen, every change through the engine; a value given in secret never comes back;
// the engine refuses a server in an untrusted project; what each server is doing is said for the
// (project, scene) asked about; signing in goes through the settings extension and its own route,
// never the engine's methods directly.
//
// The server is one whose command does not exist: what matters here is what is written and what
// the engine says of it, and a server that cannot start is said to have failed, with why. Every
// test gives back what it wrote — other suites share this backend.

import { INVALID_PARAMS, REFUSED } from "@harnix/core/protocol";
import { assert, defineSuite, sleep, type TestClient } from "@harnix/testkit";

const COMMAND = "/nonexistent/harnix-live-mcp-server";
const SECRET = "sk-live-mcp-secret-0123456789";

type Server = { name: string; tier: string; enabled: boolean; transport: string; args: string[]; env: Record<string, { kind: string; var?: string; masked?: string }>; status?: { state: string; error?: string } };

const stdio = (args: string[] = []) => ({
  transport: "stdio",
  command: COMMAND,
  args,
  env: { TOKEN: { secret: SECRET }, HOME_DIR: { var: "HOME" } },
});

async function servers(client: TestClient, params: Record<string, unknown> = {}): Promise<Server[]> {
  return (await client.call("settings.mcp.list", params)).servers;
}

const named = (list: Server[], name: string, tier = "global") => list.find((s) => s.name === name && s.tier === tier);

/** Wait for the engine to say what it makes of a server. */
async function settled(client: TestClient, name: string): Promise<Server | undefined> {
  for (let i = 0; i < 30; i++) {
    const server = named(await servers(client), name);
    if (server?.status && server.status.state !== "connecting") return server;
    await sleep(500);
  }
  return named(await servers(client), name);
}

async function giveBack(client: TestClient, name: string, tier = "global", extra: Record<string, unknown> = {}): Promise<void> {
  await client.call("settings.mcp.remove", { tier, name, ...extra }).catch(() => undefined);
}

defineSuite("settings-mcp", { feature: "settings" }, {
  tests: {
    "a server is added to the global tier, its secret kept out of every answer, and its failure said": async ({ client }) => {
      const name = "live-added";
      const added = await client.call("settings.mcp.add", { tier: "global", name, config: stdio(["--flag"]) });
      try {
        assert(/settings\.json$/.test(added.written ?? ""), `written to ${added.written}`);
        assert(!JSON.stringify(added).includes(SECRET), "the secret came back");
        const server = named(added.servers, name);
        assert(server && server.enabled && server.transport === "stdio" && server.args[0] === "--flag", JSON.stringify(server));
        assert(server.env.TOKEN.kind === "secret" && server.env.TOKEN.masked === "***6789", JSON.stringify(server.env));
        assert(server.env.HOME_DIR.kind === "env" && server.env.HOME_DIR.var === "HOME", JSON.stringify(server.env));
        // The engine tried it: a command that is not there is a failure, with why.
        const after = await settled(client, name);
        assert(after?.status?.state === "failed" && after.status.error, `the engine says ${JSON.stringify(after?.status)}`);
        const { servers: engine } = await client.call("engine.mcp.status");
        assert(engine.some((s: { name: string }) => s.name === name), "the engine does not list it");
      } finally {
        await giveBack(client, name);
      }
      assert(!named(await servers(client), name), "not removed");
      const { servers: engine } = await client.call("engine.mcp.status");
      assert(!engine.some((s: { name: string }) => s.name === name), "the engine still lists it");
    },

    "a server is changed with its secret kept, switched off and on, and removed": async ({ client }) => {
      const name = "live-changed";
      await client.call("settings.mcp.add", { tier: "global", name, config: stdio() });
      try {
        // The page never had the secret: `keep` is how it says leave it.
        const changed = await client.call("settings.mcp.update", {
          tier: "global",
          name,
          config: { transport: "stdio", command: COMMAND, args: ["--second"], env: { TOKEN: { keep: true } } },
        });
        const server = named(changed.servers, name)!;
        assert(server.args[0] === "--second" && server.env.TOKEN.masked === "***6789" && !server.env.HOME_DIR, JSON.stringify(server));
        // Off: still in the tier's file, its switch beside it, and the engine does not start it.
        const off = named((await client.call("settings.mcp.enable", { tier: "global", name, enabled: false })).servers, name)!;
        assert(off.enabled === false && off.env.TOKEN.masked === "***6789", JSON.stringify(off));
        assert(!off.status || off.status.state === "disabled", JSON.stringify(off.status));
        const { servers: engine } = await client.call("engine.mcp.status");
        const held = engine.find((s: { name: string }) => s.name === name);
        assert(!held || held.state === "disabled", `the engine has a server switched off as ${JSON.stringify(held)}`);
        // Changed while off: it stays off.
        const still = named((await client.call("settings.mcp.update", { tier: "global", name, config: { ...stdio(["--second"]), env: { TOKEN: { keep: true } } } })).servers, name)!;
        assert(still.enabled === false, JSON.stringify(still));
        // On: as it was, its secret with it.
        const on = named((await client.call("settings.mcp.enable", { tier: "global", name, enabled: true })).servers, name)!;
        assert(on.enabled && on.args[0] === "--second" && on.env.TOKEN.masked === "***6789", JSON.stringify(on));
        // Connected again alone: the engine answers for this one server.
        const again = named((await client.call("settings.mcp.reconnect", { name })).servers, name)!;
        assert(again.status && again.status.state !== "disabled", JSON.stringify(again));
      } finally {
        await giveBack(client, name);
      }
      assert(!named(await servers(client), name), "not removed");
    },

    "a scene's server is written into the scene tier and taken out of it, through the engine": async ({ client }) => {
      const name = "live-scene";
      const listed = await client.call("settings.mcp.list", {});
      assert(typeof listed.scene === "string" && listed.paths.scene, `no scene tier here: ${JSON.stringify(listed)}`);
      const added = await client.call("settings.mcp.add", { tier: "scene", name, config: stdio() });
      try {
        assert(/settings\.json$/.test(added.written ?? ""), `written to ${added.written}`);
        assert(named(added.servers, name, "scene"), JSON.stringify(added.servers));
        assert(!JSON.stringify(added).includes(SECRET), "the secret came back");
      } finally {
        await giveBack(client, name, "scene");
      }
      assert(!named(await servers(client), name, "scene"), "not removed");
    },

    "the statuses are a (project, scene)'s, said with which, and asking does not start a pair": async ({ client }) => {
      const at = { project_root: client.project };
      const listed = await client.call("settings.mcp.list", at);
      assert(listed.project_root === client.project && typeof listed.scene === "string", JSON.stringify(listed));
      assert(typeof listed.loaded === "boolean", `loaded is ${JSON.stringify(listed.loaded)}`);
      // The suite's project is not trusted: its own servers, if any, are said so and not started.
      for (const server of listed.servers.filter((s: Server) => s.tier === "project")) {
        assert(!server.status || ["untrusted", "not_started", "disabled", "denied", "failed"].includes(server.status.state), JSON.stringify(server));
      }
      // Nothing connected for a pair: nothing to connect again.
      if (listed.loaded === false && listed.servers.length) {
        const e = await client.refused("settings.mcp.reconnect", { name: listed.servers[0].name, ...at });
        assert(e && e.code === INVALID_PARAMS, JSON.stringify(e));
      }
    },

    "what a form says wrong is refused by name, and nothing is written": async ({ client }) => {
      const before = await servers(client);
      const refused = async (params: Record<string, unknown>, pattern: RegExp) => {
        const e = await client.refused("settings.mcp.add", params);
        assert(e && e.code === INVALID_PARAMS && pattern.test(e.message), `${JSON.stringify(params)}: ${JSON.stringify(e)}`);
      };
      await refused({ tier: "global", name: "a__b", config: stdio() }, /server name/);
      await refused({ tier: "global", name: "x", config: { transport: "pigeon" } }, /transport/);
      await refused({ tier: "global", name: "x", config: { transport: "http", url: "ftp://example.com/mcp" } }, /http or https/);
      await refused({ tier: "global", name: "x", config: { transport: "stdio", command: COMMAND, env: { TOKEN: {} } } }, /one of/);
      await refused({ tier: "nowhere", name: "x", config: stdio() }, /not a tier/);
      await refused(
        { tier: "project", project_root: client.project, name: "x", config: { transport: "stdio", command: COMMAND, env: { T: { var: "HOME" } } } },
        /engine's environment/,
      );
      const absent = await client.refused("settings.mcp.update", { tier: "global", name: "live-not-there", config: stdio() });
      assert(absent && absent.code === INVALID_PARAMS && /no server named/.test(absent.message), JSON.stringify(absent));
      assert(JSON.stringify(await servers(client)) === JSON.stringify(before), "a refused change wrote something");
    },

    "a project's server goes through the engine, which refuses an untrusted project": async ({ client }) => {
      // The suite's project is not trusted: a server written into its tier would not start, and the
      // engine writes nothing.
      const at = { project_root: client.project };
      const e = await client.refused("settings.mcp.add", { tier: "project", name: "live-project", config: { transport: "stdio", command: COMMAND }, ...at });
      try {
        assert(e && e.code !== REFUSED && /trust/i.test(e.message), `answered ${JSON.stringify(e)}`);
        assert(!named(await servers(client, at), "live-project", "project"), "written all the same");
      } finally {
        await giveBack(client, "live-project", "project", at);
      }
    },

    "the engine's MCP methods stay refused to a device; signing in is the settings extension's": async ({ client, backend }) => {
      for (const method of ["engine.mcp.addServer", "engine.mcp.updateServer", "engine.mcp.removeServer", "engine.mcp.setEnabled", "engine.mcp.reconnect", "engine.mcp.auth.start", "engine.mcp.auth.complete", "engine.mcp.auth.logout"]) {
        const e = await client.refused(method, {});
        assert(e && e.code === REFUSED, `${method}: ${JSON.stringify(e)}`);
      }
      // A process has nothing to sign in to: the engine says so.
      const name = "live-auth";
      await client.call("settings.mcp.add", { tier: "global", name, config: { transport: "stdio", command: COMMAND } });
      try {
        const e = await client.refused("settings.mcp.authorize", { name, origin: backend.url.replace(/\/$/, "") });
        assert(e && e.code !== REFUSED, `answered ${JSON.stringify(e)}`);
      } finally {
        await giveBack(client, name);
      }
      // A callback nobody is waiting for is handed to nobody, by either way back.
      const forged = await client.refused("settings.mcp.authComplete", { callback_url: `${backend.url}ext/settings/mcp/callback?code=x&state=forged` });
      assert(forged && forged.code === INVALID_PARAMS && /no sign-in is waiting/.test(forged.message), JSON.stringify(forged));
      const url = `${backend.url.replace(/\/$/, "")}/ext/settings/mcp/callback?code=x&state=forged`;
      assert((await fetch(url)).status === 401, "the route answered a request that carried nothing");
      const page = await fetch(url, { headers: { authorization: `Bearer ${backend.token}` } });
      const text = await page.text();
      assert(page.status === 400 && text.includes('data-harnix-mcp-auth="failed"') && /no sign-in is waiting/.test(text), `${page.status}: ${text}`);
      assert(page.headers.get("cache-control") === "no-store", "the callback page may be cached");
      // Giving up a sign-in nobody started, and signing out of a server with no token: each names
      // what it needs, and neither is refused outright.
      const missing = await client.refused("settings.mcp.authCancel", {});
      assert(missing && missing.code === INVALID_PARAMS && /flow_id/.test(missing.message), `answered ${JSON.stringify(missing)}`);
      const stray = await client.refused("settings.mcp.authCancel", { flow_id: "nobody-started-this" });
      assert(!stray || stray.code !== REFUSED, `answered ${JSON.stringify(stray)}`);
      const unnamed = await client.refused("settings.mcp.logout", { tier: "global" });
      assert(unnamed && unnamed.code === INVALID_PARAMS && /name/.test(unnamed.message), `answered ${JSON.stringify(unnamed)}`);
      await client.call("settings.mcp.add", { tier: "global", name, config: { transport: "stdio", command: COMMAND } });
      try {
        const out = await client.refused("settings.mcp.logout", { tier: "global", name });
        assert(!out || out.code !== REFUSED, `answered ${JSON.stringify(out)}`);
      } finally {
        await giveBack(client, name);
      }
    },

    "a session's MCP servers come with their tools and instructions, as the model sees them": async ({ client }) => {
      const { session_id } = await client.call("harnix.session.create", {});
      const context = await client.call("settings.mcp.context", { session_id });
      assert(Array.isArray(context.servers), JSON.stringify(context));
      for (const server of context.servers) {
        assert(typeof server.name === "string" && Array.isArray(server.tools), JSON.stringify(server));
        for (const tool of server.tools) assert(tool.name.startsWith(`mcp__`) && typeof tool.tokens === "number", JSON.stringify(tool));
      }
      const e = await client.refused("settings.mcp.context", {});
      assert(e && e.code === INVALID_PARAMS, JSON.stringify(e));
    },
  },
});
