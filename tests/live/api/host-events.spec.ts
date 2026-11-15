// The host-event channel: the one of the five semantics nothing was checking.
//
// Session events belong to a conversation and go to its watchers. Host events
// belong to the process — a session made or ended, an MCP server that would not
// connect — and go to **every** client, subscribed or not, because there is
// nothing to subscribe to. The interface draws banners off them.
//
// # What provokes one
//
// A session made is announced on the channel (`session_created`), and any client
// can make one: that is what the channel's own rules are checked with. An MCP
// server that fails is different. The engine connects its servers as it starts,
// before the link has subscribed to its events, and `engine.mcp.addServer` is
// refused here on purpose (concept §3.4): so a server that fails at startup is
// announced to no one, and a client learns of it from `engine.mcp.status`, which
// lists the servers that failed, with why. The first is a gap, written down; the
// second is what a client does today.

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { type Frame, type TestClient, assert, connect, defineSuite, sleep, startBackend } from "@harnix/testkit";

const SERVER = "a-server-that-cannot-start";

/** Settings the engine reads at startup, naming an MCP server whose command
 *  does not exist. Connecting it fails as the engine starts. */
function seedFailingMcp(scratch: string): void {
  const agix = join(scratch, "agix");
  mkdirSync(agix, { recursive: true });
  writeFileSync(join(agix, "settings.json"), JSON.stringify({
    mcp_servers: {
      // Valid configuration, unreachable binary.
      [SERVER]: {
        type: "stdio",
        command: "/nonexistent/definitely-not-a-real-binary",
        args: [],
      },
    },
  }, null, 2));
}

/** Host events already delivered, plus any that arrive within `timeout`. */
async function hostEvents(client: TestClient, match: (frame: Frame) => boolean, timeout = 20_000): Promise<Frame[]> {
  const seen = () => client.events.filter(
    (f) => f.method === "harnix.host_event" && match(f));
  if (seen().length) return seen();
  await client.waitFor((f) => f.method === "harnix.host_event" && match(f),
    { timeout, describe: "a host event" }).catch(() => {});
  return seen();
}

/** A session made by `maker`, as the host events of `client` announce it. */
async function announced(client: TestClient, maker: TestClient): Promise<Frame[]> {
  const { session_id } = await maker.call("harnix.session.create", {});
  return hostEvents(client, (f) => f.params?.kind === "session_created" && f.params?.session_id === session_id);
}

defineSuite("host-events", {
  feature: "sessions",
  gaps: {
    "an MCP server that fails as the engine starts is announced to no one":
      "Harnix: once a link is taken, announce the failures `engine.mcp.status` already holds — they happen before the link subscribes to the engine's events",
  },
}, {
  tests: {
    "a process-level fact reaches a client that subscribed to nothing":
      async ({ client: { topology } }) => {
        const backend = await startBackend({ env: { ANTHROPIC_API_KEY: "host-events" } });
        try {
          // The client watching attached to nothing and made nothing: another client made the session.
          const watching = await connect(backend, { topology });
          const maker = await connect(backend, { topology });
          const found = await announced(watching, maker);
          assert(found.length,
            `no host event arrived; saw ${watching.events.map((e) => e.method).join(", ") || "nothing"}`);
          watching.close();
          maker.close();
        } finally {
          await backend.dispose();
        }
      },

    "a host event is a broadcast, not a subscription": async ({ client: { topology } }) => {
      const backend = await startBackend({ env: { ANTHROPIC_API_KEY: "host-events" } });
      try {
        // Two clients, neither of which attached to anything. The defining
        // property of this semantic is that there is nothing to ask for:
        // process-level facts go to everyone who is connected.
        const first = await connect(backend, { topology });
        const second = await connect(backend, { topology });
        const { session_id } = await first.call("harnix.session.create", {});
        const clients: [string, TestClient][] = [["first", first], ["second", second]];
        for (const [name, client] of clients) {
          const found = await hostEvents(client, (f) => f.params?.kind === "session_created" && f.params?.session_id === session_id);
          assert(found.length, `the ${name} client, which subscribed to nothing, was not told`);
        }
        first.close();
        second.close();
      } finally {
        await backend.dispose();
      }
    },

    "the host face carries host events and the session face does not":
      async ({ client: { topology } }) => {
        // Only split streams can be wrong about this: there, the two kinds
        // travel on two separate downloads, and a host event delivered on the
        // session face would mean a read-only client watching one
        // conversation is being handed process-level facts it did not ask for
        // — and, worse, that the faces are interchangeable, which is the
        // thing the split exists to disprove.
        if (topology !== "split_streams") return;
        const backend = await startBackend({ env: { ANTHROPIC_API_KEY: "host-events" } });
        try {
          const client = await connect(backend, { topology });
          const found = await announced(client, client);
          assert(found.length, "no host event arrived on split streams");
          // `harnix.event` is the session face's method. A host event wearing
          // that method would have come down the wrong stream.
          const misrouted = client.events.filter(
            (f) => f.method === "harnix.event" && f.params?.kind === "session_created");
          assert(!misrouted.length, "a host event arrived as a session event");
          client.close();
        } finally {
          await backend.dispose();
        }
      },

    "an MCP server that fails as the engine starts is announced to no one":
      async ({ client: { topology } }) => {
        // Not an assertion that this is right — it is the gap, written down.
        const backend = await startBackend({
          env: { ANTHROPIC_API_KEY: "host-events" },
          seed: seedFailingMcp,
        });
        try {
          const client = await connect(backend, { topology });
          await sleep(3_000);
          const told = client.events.filter((f) => f.method === "harnix.host_event" && f.params?.kind === "mcp_connect_failed");
          assert(!told.length, "the failure is announced now — assert that it names the server and why, and drop the @gap");
          client.close();
        } finally {
          await backend.dispose();
        }
      },

    "a client learns of a server that failed from the engine's status":
      async ({ client: { topology } }) => {
        const backend = await startBackend({
          env: { ANTHROPIC_API_KEY: "host-events" },
          seed: seedFailingMcp,
        });
        try {
          const client = await connect(backend, { topology });
          const { servers } = await client.call("engine.mcp.status");
          const row = servers.find((s: { name: string }) => s.name === SERVER);
          // The reason travels with it. A banner that says "an MCP server
          // failed" and not which one, or why, is a banner nobody can act on.
          assert(row?.state === "failed" && row.error, `engine.mcp.status says ${JSON.stringify(row)}`);
          client.close();
        } finally {
          await backend.dispose();
        }
      },
  },
});
