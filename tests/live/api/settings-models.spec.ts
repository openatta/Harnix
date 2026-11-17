// Model settings: the engine's model parameters, its routes by task, and a provider's key and
// thinking as the settings page reads and writes them.
//
// Every key is one the engine reads (agix/docs/schemas/settings.schema.json). The models a field
// offers are the configured providers' own; what an endpoint serves is asked through
// `settings.listModels`, and the endpoints here are not there, so what is checked is that the
// asking fails as the engine says, and that nothing about the key comes back with it.

import { isDeepStrictEqual } from "node:util";

import { INVALID_PARAMS, REFUSED } from "@harnix/core/protocol";
import { assert, connect, defineSuite, expect, launchFor, startHarnix, test } from "@harnix/testkit";

type Field = { key: string; type: string; section: string; options?: string[]; suggestions?: string[]; routes?: { provider: string; default_model?: string | null; models?: string[] }[]; tiers: Record<string, unknown>; effective: unknown };

const TASKS = ["main", "subagent", "compact", "memory", "title", "web_search", "web_fetch"];

/** A provider the tests route to: the engine builds a client for it without calling it. */
const PROVIDER = "test-models";
const CONFIG = { api_type: "openai_compatible", base_url: "https://example.invalid/v1", api_key: "sk-secret-value-wxyz", default_model: "m-default", models: ["m-default", "m-other"] };

defineSuite("settings-models", { feature: "settings" }, {
  tests: {
    "the model parameters and the routes by task are described as the engine reads them": async ({ client }) => {
      const described = await client.call("settings.describe", {});
      const field = (key: string): Field => {
        const found = described.fields.find((f: Field) => f.key === key);
        assert(found, `${key} is not described`);
        return found;
      };
      assert(JSON.stringify(field("model.effort").options) === JSON.stringify(["low", "medium", "high", "xhigh", "max"]), JSON.stringify(field("model.effort")));
      assert(field("model.thinking_mode").type === "select", JSON.stringify(field("model.thinking_mode")));
      assert(field("model.max_output_tokens").type === "number", JSON.stringify(field("model.max_output_tokens")));
      const fallback = field("model.fallback_model");
      assert(fallback.type === "text" && Array.isArray(fallback.suggestions), JSON.stringify(fallback));
      for (const task of TASKS) {
        const route = field(`task_models.${task}`);
        assert(route.type === "route" && route.section === "task-models" && Array.isArray(route.routes), JSON.stringify(route));
      }
      // `main` is the conversation's own route.
      assert(field("task_models.main").section === "task-models", JSON.stringify(field("task_models.main")));
    },

    "a provider's models are offered to the model fields and to every route": async ({ client }) => {
      await client.call("settings.setProvider", { provider_id: PROVIDER, config: CONFIG });
      try {
        const described = await client.call("settings.describe", {});
        const field = (key: string): Field => described.fields.find((f: Field) => f.key === key);
        for (const key of ["model.fallback_model", "model.model_name"]) {
          const offered = field(key).suggestions ?? [];
          assert(offered.includes("m-default") && offered.includes("m-other"), `${key} offers ${JSON.stringify(offered)}`);
        }
        const route = field("task_models.title").routes?.find((r) => r.provider === PROVIDER);
        assert(route && route.default_model === "m-default" && JSON.stringify(route.models) === JSON.stringify(["m-default", "m-other"]), JSON.stringify(field("task_models.title")));
      } finally {
        await client.call("settings.setProvider", { provider_id: PROVIDER, delete: true });
      }
    },

    "a task is routed to a provider and a model of it, and a route to a provider that is not configured is refused by name": async ({ client }) => {
      await client.call("settings.setProvider", { provider_id: PROVIDER, config: CONFIG });
      try {
        const written = await client.call("settings.write", {
          tier: "global",
          changes: { "task_models.title": { provider: PROVIDER, model: "m-other" }, "task_models.compact": PROVIDER },
        });
        const field = (key: string): Field => written.fields.find((f: Field) => f.key === key);
        // Compared as values: the tier's file comes back with its keys in their own order.
        assert(isDeepStrictEqual(field("task_models.title").tiers.global, { provider: PROVIDER, model: "m-other" }), JSON.stringify(field("task_models.title")));
        assert(field("task_models.compact").tiers.global === PROVIDER, JSON.stringify(field("task_models.compact")));

        const before = await client.call("settings.describe", {});
        const e = await client.refused("settings.write", { tier: "global", changes: { "task_models.memory": { provider: "not-configured" } } });
        assert(e && e.code === INVALID_PARAMS, `answered ${JSON.stringify(e)}`);
        assert(e.data?.errors?.[0]?.key === "task_models.memory", `named ${JSON.stringify(e.data)}`);
        const shape = await client.refused("settings.write", { tier: "global", changes: { "task_models.memory": { provider: PROVIDER, base_url: "http://elsewhere" } } });
        assert(shape && shape.code === INVALID_PARAMS, `a route with a base_url was taken: ${JSON.stringify(shape)}`);
        const after = await client.call("settings.describe", {});
        assert(after.revision === before.revision, "a refused route was written");
      } finally {
        await client.call("settings.write", { tier: "global", changes: { "task_models.title": null, "task_models.compact": null } });
        await client.call("settings.setProvider", { provider_id: PROVIDER, delete: true });
      }
    },

    "effort, thinking, the output limit and the fallback model are written and given back": async ({ client }) => {
      const changes = { "model.effort": "low", "model.thinking_mode": "off", "model.max_output_tokens": 16000, "model.fallback_model": "m-fallback" };
      const written = await client.call("settings.write", { tier: "global", changes });
      try {
        for (const [key, value] of Object.entries(changes)) {
          const field: Field = written.fields.find((f: Field) => f.key === key);
          assert(field.tiers.global === value && field.effective === value, `${key}: ${JSON.stringify(field)}`);
        }
        const e = await client.refused("settings.write", { tier: "global", changes: { "model.effort": "extreme", "model.thinking_mode": "sometimes" } });
        assert(e && e.code === INVALID_PARAMS, `answered ${JSON.stringify(e)}`);
        const named = (e.data?.errors ?? []).map((x: { key: string }) => x.key).sort();
        assert(JSON.stringify(named) === JSON.stringify(["model.effort", "model.thinking_mode"]), `named ${JSON.stringify(named)}`);
      } finally {
        await client.call("settings.write", { tier: "global", changes: Object.fromEntries(Object.keys(changes).map((k) => [k, null])) });
      }
    },

    "a provider's key is read back as its last four, and the variable it may come from by name": async ({ client }) => {
      await client.call("settings.setProvider", { provider_id: PROVIDER, config: { ...CONFIG, api_key_env: "HARNIX_TEST_MODELS_KEY" } });
      try {
        const described = await client.call("settings.describe", {});
        const provider = described.providers?.providers?.[PROVIDER];
        assert(provider?.api_key === "***wxyz", `read back as ${JSON.stringify(provider)}`);
        assert(provider.api_key_env === "HARNIX_TEST_MODELS_KEY", JSON.stringify(provider));
        // Whether the engine has a key for it, as it says: one is stored.
        assert(provider.credential_present === true, `credential_present: ${JSON.stringify(provider)}`);
        assert(!JSON.stringify(described).includes("sk-secret-value"), "the key came back in the clear");
      } finally {
        await client.call("settings.setProvider", { provider_id: PROVIDER, delete: true });
      }
    },

    "a provider's models are objects with their own limits, and their shape is checked first": async ({ client }) => {
      const models = ["m-default", { id: "m-other", max_output: 4096, context_window: 64000 }];
      await client.call("settings.setProvider", { provider_id: PROVIDER, config: { ...CONFIG, models } });
      try {
        const described = await client.call("settings.describe", {});
        assert(isDeepStrictEqual(described.providers?.providers?.[PROVIDER]?.models, models), JSON.stringify(described.providers?.providers?.[PROVIDER]));
        // Offered by id, whichever way it is written.
        const route = described.fields.find((f: Field) => f.key === "task_models.title").routes?.find((r: { provider: string }) => r.provider === PROVIDER);
        assert(JSON.stringify(route?.models) === JSON.stringify(["m-default", "m-other"]), JSON.stringify(route));
        const e = await client.refused("settings.setProvider", { provider_id: PROVIDER, config: { models: [{ id: "m", max_output: 0 }] } });
        assert(e && e.code === INVALID_PARAMS && /max_output/.test(e.message), `answered ${JSON.stringify(e)}`);
      } finally {
        await client.call("settings.setProvider", { provider_id: PROVIDER, delete: true });
      }
    },

    "an endpoint that cannot be asked for its models says so, and nothing of the key comes back": async ({ client }) => {
      await client.call("settings.setProvider", { provider_id: PROVIDER, config: CONFIG });
      try {
        const e = await client.refused("settings.listModels", { provider: PROVIDER });
        assert(e && e.code !== INVALID_PARAMS, `answered ${JSON.stringify(e)}`);
        assert(!JSON.stringify(e).includes("sk-secret-value"), "the key came back with the failure");
        const unknown = await client.refused("settings.listModels", { provider: "not-configured" });
        assert(unknown && unknown.code === INVALID_PARAMS, `answered ${JSON.stringify(unknown)}`);
        // The engine's own method stays off a device.
        const direct = await client.refused("engine.config.listModels", { provider: PROVIDER });
        assert(direct && direct.code === REFUSED, `answered ${JSON.stringify(direct)}`);
      } finally {
        await client.call("settings.setProvider", { provider_id: PROVIDER, delete: true });
      }
    },

    "an OpenAI-compatible provider's thinking switch is taken, and one the engine does not know is not": async ({ client }) => {
      await client.call("settings.setProvider", { provider_id: PROVIDER, config: { ...CONFIG, thinking_toggle: "deepseek", reasoning_passback: true } });
      try {
        const provider = (await client.call("settings.describe", {})).providers?.providers?.[PROVIDER];
        assert(provider?.thinking_toggle === "deepseek" && provider.reasoning_passback === true, JSON.stringify(provider));
        const e = await client.refused("settings.setProvider", { provider_id: PROVIDER, config: { thinking_toggle: "always" } });
        assert(e && e.code === INVALID_PARAMS && /thinking_toggle/.test(e.message), `answered ${JSON.stringify(e)}`);
      } finally {
        await client.call("settings.setProvider", { provider_id: PROVIDER, delete: true });
      }
    },
  },
});

// Whether the engine has a key for a provider is its own word (`credential_present`), from what is
// stored or the variable named. A provider it would have no key for never gets that far: the page's
// write is tried first and one without a key does not route — nor does an engine start with one,
// and a hand edit of the global tier is not read while it runs. So "missing" is the interface's to
// draw should the engine say it; what is checked here is the engine saying "present" for a key taken
// from a variable, and refusing the keyless one.
test("a provider's key from a variable is present as the engine says, and one with no key at all is refused", { tag: ["@feature:settings", "@layer:api"] }, async () => {
  const { api_key: _key, ...keyless } = CONFIG;
  const backend = await startHarnix(launchFor("none", { env: { HARNIX_TEST_MODELS_KEY: "sk-from-the-environment-abcd" } }));
  try {
    const client = await connect(backend);
    const e = await client.refused("settings.setProvider", { provider_id: PROVIDER, config: { ...keyless, api_key_env: "HARNIX_TEST_MODELS_KEY_NOT_SET" } });
    expect(e?.code, JSON.stringify(e)).toBe(INVALID_PARAMS);
    expect(e?.message).toContain("api_key");
    expect((await client.call("settings.describe", {})).providers?.providers?.[PROVIDER]).toBeUndefined();
    await client.call("settings.setProvider", { provider_id: PROVIDER, config: { ...keyless, api_key_env: "HARNIX_TEST_MODELS_KEY" } });
    const provider = (await client.call("settings.describe", {})).providers?.providers?.[PROVIDER];
    expect(provider?.credential_present, JSON.stringify(provider)).toBe(true);
    expect(provider?.api_key_env).toBe("HARNIX_TEST_MODELS_KEY");
    expect(JSON.stringify(provider)).not.toContain("from-the-environment");
    client.close();
  } finally {
    await backend.dispose();
  }
});
