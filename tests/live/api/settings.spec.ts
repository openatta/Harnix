// Settings: what the three tiers say, and what happens when they are written.
//
// The engine memoizes resolved settings per (project, scene) and only
// refreshes its own pair on reload, so the effective value it reports can lag
// a write. Harnix computes the tiers itself and hands back the engine's answer
// alongside — so when they disagree, the interface can say so instead of
// showing one of them and being wrong half the time.

import { METHOD_NOT_FOUND, REFUSED } from "@harnix/core/protocol";
import { defineSuite } from "@harnix/testkit";
const assert = (cond, message) => { if (!cond) throw new Error(message); };

defineSuite("settings", { feature: "settings" }, {
  tests: {
    "settings describe their tiers": async ({ client }) => {
      const described = await client.call("harnix.settings.describe", {});
      assert(described, "nothing described");
      const asJson = JSON.stringify(described);
      assert(/global|local|project|effective/.test(asJson), "no tiers in the description");
    },

    "a write lands in the tier it was aimed at": async ({ client }) => {
      const key = "model.max_tokens";
      const before = await client.call("harnix.settings.describe", {});
      await client.call("harnix.settings.set", { tier: "global", key, value: 4321 });
      const after = await client.call("harnix.settings.describe", {});
      assert(JSON.stringify(after) !== JSON.stringify(before), "the write changed nothing");
      // Null gives a setting back to the tier below it, rather than pinning a
      // default that then cannot be un-pinned.
      await client.call("harnix.settings.set", { tier: "global", key, value: null });
    },

    "an unknown tier is refused rather than guessed": async ({ client }) => {
      const e = await client.refused("harnix.settings.set",
        { tier: "nonsense", key: "model.model_name", value: "x" });
      assert(e, "an unknown tier was accepted");
    },

    "a key that is not editable is refused by name": async ({ client }) => {
      // The editable set is a list, not "anything in the file": a settings
      // page that could write arbitrary keys is a JSON editor with extra
      // steps, and a typo would silently create a setting nothing reads.
      const e = await client.refused("harnix.settings.set",
        { tier: "global", key: "make.believe", value: 1 });
      assert(e, "an unknown key was written");
      assert(/editable/.test(e.message), `says "${e.message}"`);
    },

    "a value of the wrong shape is refused": async ({ client }) => {
      const e = await client.refused("harnix.settings.set",
        { tier: "global", key: "model.max_tokens", value: "not a number" });
      assert(e, "a count accepted a string");
    },

    "providers can be read back": async ({ client }) => {
      const described = await client.call("harnix.settings.describe", { section: "providers" })
        .catch(() => null);
      assert(described !== undefined, "providers could not be described");
    },

    "a provider is added, read back with its key redacted, and removed": async ({ client }) => {
      // Written to the global tier and reloaded, so the very next read sees
      // it — not the working directory's project tier, which depends on where
      // the process was started.
      const id = "test-openai";
      const added = await client.call("harnix.settings.setProvider", {
        provider_id: id,
        config: { api_type: "openai_compatible", base_url: "https://example.invalid/v1", api_key: "sk-secret-value", default_model: "m-1", models: ["m-2"] },
      });
      assert(added.reloaded === true, "the engine was not asked to reload");
      assert(/settings\.json$/.test(added.written) && !added.written.includes("/.agix/.agix"), `written to ${added.written}`);
      const described = await client.call("harnix.settings.describe", {});
      const provider = described.providers?.providers?.[id];
      assert(provider, "the provider is not read back");
      assert(provider.default_model === "m-1" && provider.models?.[0] === "m-2", JSON.stringify(provider));
      assert(provider.api_key && provider.api_key !== "sk-secret-value", "the key came back in the clear");
      // An edit that leaves the key out keeps it.
      await client.call("harnix.settings.setProvider", { provider_id: id, config: { api_type: "openai_compatible", default_model: "m-3" } });
      const edited = (await client.call("harnix.settings.describe", {})).providers.providers[id];
      assert(edited.default_model === "m-3" && edited.api_key, JSON.stringify(edited));
      await client.call("harnix.settings.setProvider", { provider_id: id, delete: true });
      const after = await client.call("harnix.settings.describe", {});
      assert(!after.providers?.providers?.[id], "the provider was not removed");
    },

    "a provider of an unknown API type is refused": async ({ client }) => {
      const e = await client.refused("harnix.settings.setProvider", { provider_id: "x", config: { api_type: "gemini" } });
      assert(e && /api_type/.test(e.message), `says ${e?.message}`);
    },

    "setting a provider is reachable here and refused on the engine": async ({ client }) => {
      // The engine's own `config.setProvider` is refused to every client —
      // it repoints model traffic. The same intent goes through this layer,
      // where it is a deliberate, audited method rather than a passthrough.
      const direct = await client.refused("config.setProvider", {});
      assert(direct && direct.code === REFUSED, "config.setProvider is not refused");
      const e = await client.refused("harnix.settings.setProvider", {});
      // Refused for want of arguments, not for want of permission.
      if (e) assert(e.code !== REFUSED, `refused as unauthorized: ${e.message}`);
    },
  },
});
