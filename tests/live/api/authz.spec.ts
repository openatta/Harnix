// The one admission point.
//
// Everything not written down is refused, and everything refused says why.
// "Unknown method" and "you may not" are different answers: a client that
// cannot tell them apart reports the wrong bug.

import { METHOD_NOT_FOUND, REFUSED } from "@harnix/core/protocol";
import { assert, defineSuite } from "@harnix/testkit";

defineSuite("authz", { feature: "authz" }, {
  tests: {
    "a subject can ask what it may call": async ({ client }) => {
      const { methods, subject } = await client.call("harnix.reachable");
      assert(Array.isArray(methods) && methods.length > 20, `${methods?.length} methods`);
      assert(subject.kind === "device", `subject is ${subject.kind}`);
      assert(methods.includes("harnix.session.send"), "harnix.session.send is not reachable");
    },

    "an unlisted method is not found": async ({ client }) => {
      const e = await client.refused("harnix.noSuchThing");
      assert(e, "an unlisted method answered");
      assert(e.code === METHOD_NOT_FOUND, `code ${e.code}`);
    },

    "credential and endpoint methods are refused with a reason": async ({ client }) => {
      for (const method of ["engine.config.setProvider", "engine.config.set", "engine.daemon.shutdown", "engine.import.run"]) {
        const e = await client.refused(method, {});
        assert(e, `${method} was allowed`);
        // Refused, not merely absent — the distinction is the whole point.
        assert(e.code === REFUSED, `${method}: code ${e.code}`);
        assert(e.message.length > 20, `${method}: no reason given`);
      }
    },

    "the hub's own semantics are not reachable around it": async ({ client }) => {
      const reachedBy: [string, RegExp][] = [
        ["engine.session.run_turn", /harnix\.session\.send/],
        ["engine.session.subscribe", /harnix\.session\.attach/],
        ["engine.session.unsubscribe", /harnix\.session\.detach/],
        ["engine.daemon.subscribeEvents", /host events/i],
      ];
      for (const [method, expect] of reachedBy) {
        const e = await client.refused(method, {});
        assert(e, `${method} was allowed`);
        assert(expect.test(e.message), `${method}: says "${e.message}"`);
      }
    },

    "adding an MCP server stays refused while installing a plugin does not": async ({ client }) => {
      // Installing a package is this person's decision: it arrives with a
      // manifest, a capability declaration and a disclosure. Adding an MCP
      // server configures a subprocess-spawning tool with none of the three,
      // so it is refused here and named in the reason.
      const mcp = await client.refused("engine.mcp.addServer", {});
      assert(mcp && mcp.code === REFUSED, "engine.mcp.addServer is not refused");

      const { methods } = await client.call("harnix.reachable");
      for (const method of ["plugins.install", "plugins.list"]) {
        assert(methods.includes(method), `${method} is not reachable`);
      }
    },

    "what did not take is queryable": async ({ client }) => {
      // Every extension is listed whether or not it took, and one that did not says why.
      const { extensions } = await client.call("harnix.extensions");
      assert(extensions.length > 5, `${extensions.length} extensions`);
      for (const e of extensions) {
        assert(typeof e.enabled === "boolean", `${e.id}: no enabled flag`);
        assert(e.enabled || e.reason, `${e.id} is off and does not say why`);
      }
    },
  },
});
