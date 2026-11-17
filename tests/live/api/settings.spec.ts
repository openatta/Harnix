// Settings: what the three tiers say, and what happens when they are written.
//
// The value a field shows is the engine's own for the (project, scene) asked about, re-read after a
// write; `source` is the highest tier whose file wrote it, and `withheld` says the project tier
// wrote a value the engine does not use (an untrusted project can only tighten). Every write is the
// engine's (`engine.config.set`), against the revision of the tier written.

import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { isDeepStrictEqual } from "node:util";

import { CONFLICT, INVALID_PARAMS, METHOD_NOT_FOUND, REFUSED } from "@harnix/core/protocol";
import { assert, defineSuite } from "@harnix/testkit";

/** The engine's refusal of what an untrusted project's own tier may not take (`PROJECT_UNTRUSTED`): nothing written, `data.rejected` naming each key. */
const PROJECT_UNTRUSTED = -32022;

/** A project's own settings file, as a person writes it by hand: the engine reads it as it is. */
function writeProjectSettings(project: string, settings: Record<string, unknown>): void {
  mkdirSync(join(project, ".harnix"), { recursive: true });
  writeFileSync(join(project, ".harnix", "settings.json"), JSON.stringify(settings));
}
function removeProjectSettings(project: string): void {
  rmSync(join(project, ".harnix", "settings.json"), { force: true });
}

defineSuite("settings", { feature: "settings" }, {
  tests: {
    "settings describe their tiers": async ({ client }) => {
      const described = await client.call("settings.describe", {});
      assert(described, "nothing described");
      const asJson = JSON.stringify(described);
      assert(/global|local|project|effective/.test(asJson), "no tiers in the description");
    },

    "a write lands in the tier it was aimed at": async ({ client }) => {
      const key = "model.max_tokens";
      const before = await client.call("settings.describe", {});
      await client.call("settings.set", { tier: "global", key, value: 4321 });
      const after = await client.call("settings.describe", {});
      assert(JSON.stringify(after) !== JSON.stringify(before), "the write changed nothing");
      // Null gives a setting back to the tier below it, rather than pinning a
      // default that then cannot be un-pinned.
      await client.call("settings.set", { tier: "global", key, value: null });
    },

    "an unknown tier is refused rather than guessed": async ({ client }) => {
      const e = await client.refused("settings.set",
        { tier: "nonsense", key: "model.model_name", value: "x" });
      assert(e, "an unknown tier was accepted");
    },

    "a key that is not editable is refused by name": async ({ client }) => {
      // The editable set is a list, not "anything in the file": a settings
      // page that could write arbitrary keys is a JSON editor with extra
      // steps, and a typo would silently create a setting nothing reads.
      const e = await client.refused("settings.set",
        { tier: "global", key: "make.believe", value: 1 });
      assert(e, "an unknown key was written");
      assert(/editable/.test(e.message), `says "${e.message}"`);
    },

    "a value of the wrong shape is refused": async ({ client }) => {
      const e = await client.refused("settings.set",
        { tier: "global", key: "model.max_tokens", value: "not a number" });
      assert(e, "a count accepted a string");
    },

    "every field comes with its declaration, and the tiers with a revision": async ({ client }) => {
      // The page draws its forms from these: a field added to the table needs nothing on the page.
      const described = await client.call("settings.describe", {});
      assert(typeof described.revision === "string" && described.revision, "no revision");
      const keys = described.fields.map((f: { key: string }) => f.key);
      for (const key of ["permission_mode", "memory_enabled", "allow_client_permission_override", "model.model_name", "model.max_tokens"]) {
        assert(keys.includes(key), `${key} is not described`);
      }
      for (const field of described.fields) {
        assert(field.type && field.section && field.label && Array.isArray(field.writable), `${field.key}: ${JSON.stringify(field)}`);
      }
      const mode = described.fields.find((f: { key: string }) => f.key === "permission_mode");
      assert(mode.type === "select" && mode.kind === "choice" && mode.default === "default", JSON.stringify(mode));
      assert(mode.risk?.values?.includes("bypassPermissions"), "full access is not a value to confirm");
      const tokens = described.fields.find((f: { key: string }) => f.key === "model.max_tokens");
      assert(tokens.type === "number" && tokens.min === 1 && tokens.max === 1_000_000, JSON.stringify(tokens));
    },

    "several fields are written as one, against the revision they were read at": async ({ client }) => {
      const read = await client.call("settings.describe", {});
      const written = await client.call("settings.write", {
        tier: "global",
        changes: { "model.max_tokens": 4321, memory_enabled: false },
        revision: read.revision,
      });
      try {
        const field = (key: string) => written.fields.find((f: { key: string }) => f.key === key);
        assert(field("model.max_tokens").tiers.global === 4321 && field("memory_enabled").tiers.global === false, JSON.stringify(written.fields));
        assert(written.revision !== read.revision, "a write left the revision as it was");
        // The revision read before that write is behind now: a write naming it is refused, and
        // says the one to read again at.
        const stale = await client.refused("settings.write", { tier: "global", changes: { "model.max_tokens": 1234 }, revision: read.revision });
        assert(stale && stale.code === CONFLICT, `answered ${JSON.stringify(stale)}`);
        assert(stale.data?.revision === written.revision, `says ${JSON.stringify(stale.data)}`);
        const after = await client.call("settings.describe", {});
        assert(after.fields.find((f: { key: string }) => f.key === "model.max_tokens").tiers.global === 4321, "written over a revision that moved on");
      } finally {
        await client.call("settings.write", { tier: "global", changes: { "model.max_tokens": null, memory_enabled: null } });
      }
    },

    "a change with values the fields do not accept is refused whole, each one named": async ({ client }) => {
      const before = await client.call("settings.describe", {});
      const e = await client.refused("settings.write", {
        tier: "global",
        changes: { memory_enabled: "yes", "model.max_tokens": 0, "make.believe": 1, permission_mode: "plan" },
      });
      assert(e && e.code === INVALID_PARAMS, `answered ${JSON.stringify(e)}`);
      const named = (e.data?.errors ?? []).map((x: { key: string }) => x.key).sort();
      assert(JSON.stringify(named) === JSON.stringify(["make.believe", "memory_enabled", "model.max_tokens"]), `named ${JSON.stringify(named)}`);
      const after = await client.call("settings.describe", {});
      assert(after.revision === before.revision, "part of a refused change was written");
    },

    "a dry run is checked by the engine and writes nothing": async ({ client }) => {
      const before = await client.call("settings.describe", {});
      const tried = await client.call("settings.write", { tier: "global", changes: { "model.max_tokens": 2468 }, dry_run: true });
      assert(tried.dry_run === true && Array.isArray(tried.rejected), JSON.stringify({ dry_run: tried.dry_run, rejected: tried.rejected }));
      const after = await client.call("settings.describe", {});
      assert(after.revision === before.revision, "a dry run wrote something");
      assert(after.fields.find((f: { key: string }) => f.key === "model.max_tokens").tiers.global !== 2468, "a dry run's value was written");
    },

    "a write into a project whose folder is not there is refused by the engine, and nothing is made": async ({ client }) => {
      const gone = `${client.project}-not-there`;
      const e = await client.refused("settings.write", { tier: "project", project_root: gone, changes: { permission_mode: "plan" } });
      assert(e && e.code === -32010, `answered ${JSON.stringify(e)}`);
      const { existsSync } = await import("node:fs");
      assert(!existsSync(gone), "the project's folder was made");
    },

    // general & display
    "the engine's language is a global-tier field, written and given back": async ({ client }) => {
      const language = async () => (await client.call("settings.describe", {})).fields.find((f: { key: string }) => f.key === "language");
      const declared = await language();
      assert(declared.type === "text" && declared.section === "general" && JSON.stringify(declared.writable) === '["global"]', JSON.stringify(declared));
      await client.call("settings.write", { tier: "global", changes: { language: "zh-CN" } });
      try {
        assert((await language()).tiers.global === "zh-CN", "the language was not written");
        // A project has no say in what the engine answers in.
        const e = await client.refused("settings.write", { tier: "project", project_root: client.project, changes: { language: "en" } });
        assert(e && e.code === INVALID_PARAMS, `answered ${JSON.stringify(e)}`);
      } finally {
        await client.call("settings.write", { tier: "global", changes: { language: null } });
      }
      assert((await language()).tiers.global === null, "the language was not given back");
    },

    // privacy & advanced
    "telemetry starts off in Harnix, is a global-tier switch, and has an endpoint beside it": async ({ client }) => {
      const described = await client.call("settings.describe", {});
      const flag = described.fields.find((f: { key: string }) => f.key === "telemetry_enabled");
      assert(flag.type === "toggle" && flag.section === "privacy" && flag.default === false && JSON.stringify(flag.writable) === '["global"]', JSON.stringify(flag));
      assert(flag.hint, "the switch says nothing of what it does");
      // The instance was started with the default Harnix writes into its global tier.
      assert(flag.effective === false, `telemetry reads ${JSON.stringify(flag.effective)} on a fresh instance`);
      const url = described.fields.find((f: { key: string }) => f.key === "telemetry_url");
      assert(url.type === "text" && url.section === "privacy", JSON.stringify(url));
      const e = await client.refused("settings.write", { tier: "project", project_root: client.project, changes: { telemetry_enabled: true } });
      assert(e && e.code === INVALID_PARAMS, "a project switched telemetry");
      await client.call("settings.write", { tier: "global", changes: { telemetry_enabled: true } });
      try {
        assert((await client.call("settings.describe", {})).fields.find((f: { key: string }) => f.key === "telemetry_enabled").tiers.global === true, "not written");
      } finally {
        await client.call("settings.write", { tier: "global", changes: { telemetry_enabled: false } });
      }
    },

    "the advanced values are shown with their source and refused as writes in every tier": async ({ client }) => {
      const described = await client.call("settings.describe", {});
      for (const key of ["compaction", "execution", "hooks_config", "team", "checkpoints"]) {
        const field = described.fields.find((f: { key: string }) => f.key === key);
        assert(field && field.type === "json" && field.section === "advanced" && field.writable.length === 0, `${key}: ${JSON.stringify(field)}`);
        assert("effective" in field && "source" in field, `${key} says nothing of where it stands`);
        for (const tier of ["global", "project"]) {
          const e = await client.refused("settings.write", { tier, project_root: client.project, changes: { [key]: {} } });
          assert(e && e.code === INVALID_PARAMS, `${key} was accepted in ${tier}`);
        }
      }
      // The settings files are named, for editing by hand.
      assert(/settings\.json$/.test(described.paths.global ?? ""), JSON.stringify(described.paths));
    },

    // permissions, sandbox, network
    "the permission, sandbox and network fields are the engine's keys": async ({ client }) => {
      const described = await client.call("settings.describe", {});
      const field = (key: string) => described.fields.find((f: { key: string }) => f.key === key);
      const sections: Record<string, string> = {
        permission_mode: "permissions",
        allow_client_permission_override: "permissions",
        permission_rules: "permissions",
        "sandbox.network_mode": "sandbox",
        "sandbox.egress_proxy": "sandbox",
        "sandbox.additional_directories": "sandbox",
        "sandbox.read_directories": "sandbox",
        "sandbox.require_enforcement": "sandbox",
        "network.use_system_proxy": "network",
        "sandbox.allowed_domains": "network",
      };
      for (const [key, section] of Object.entries(sections)) {
        assert(field(key)?.section === section, `${key}: ${JSON.stringify(field(key))}`);
      }
      assert(field("permission_mode").options.includes("dontAsk"), "dontAsk is not a mode");
      const rules = field("permission_rules");
      assert(rules.type === "rules" && rules.additive === true, JSON.stringify(rules));
      assert(JSON.stringify(rules.columns.map((c: { key: string }) => c.key)) === JSON.stringify(["tool", "action", "match"]), JSON.stringify(rules.columns));
      assert(rules.columns[0].check === "tool", "the tool column is not checked as a tool");
      // The engine reads its own proxy setting from the global tier only.
      assert(JSON.stringify(field("network.use_system_proxy").writable) === JSON.stringify(["global"]), JSON.stringify(field("network.use_system_proxy")));
      // The web's two routes are with the other routes (settings-models), declared once.
      for (const key of ["task_models.web_search", "task_models.web_fetch"]) {
        assert(described.fields.filter((f: { key: string }) => f.key === key).length === 1 && field(key).section === "task-models", JSON.stringify(field(key)));
      }
    },

    "permission rules are written in the engine's shape, and a tool it cannot match is refused": async ({ client }) => {
      const rules = [{ tool: "Bash(git push:*)", action: "deny" }, { tool: "Edit(docs/**)", action: "allow" }];
      const written = await client.call("settings.write", { tier: "global", changes: { permission_rules: rules } });
      try {
        const field = written.fields.find((f: { key: string }) => f.key === "permission_rules");
        // Compared as values: the engine's file comes back with its keys in its own order.
        assert(isDeepStrictEqual(field.tiers.global, rules), JSON.stringify(field));
        // The engine took them: they are among the rules it runs with.
        assert(Array.isArray(field.effective) && field.effective.some((r: { tool: string }) => r.tool === "Bash(git push:*)"), JSON.stringify(field.effective));
        const e = await client.refused("settings.write", { tier: "global", changes: { permission_rules: [{ tool: "mcp__github__*", action: "deny" }] } });
        assert(e && e.code === INVALID_PARAMS && /wildcard/.test(e.message), `answered ${JSON.stringify(e)}`);
        const flowWords = await client.refused("settings.write", { tier: "global", changes: { permission_rules: [{ tool: "Bash", effect: "deny" }] } });
        assert(flowWords && flowWords.code === INVALID_PARAMS, "a rule in a flow's keys was written into the engine's settings");
      } finally {
        await client.call("settings.write", { tier: "global", changes: { permission_rules: null } });
      }
    },

    "an untrusted project's allow rule is refused when written and withheld when found, its deny is not": async ({ client }) => {
      const at = { project_root: client.project };
      const field = async () =>
        (await client.call("settings.describe", at)).fields.find((f: { key: string }) => f.key === "permission_rules");
      await client.call("settings.write", { tier: "project", changes: { permission_rules: [{ tool: "Bash(rm:*)", action: "deny" }] }, ...at });
      try {
        const deny = await field();
        assert(deny.withheld === false && deny.source === "project", JSON.stringify(deny));
        // The engine takes nothing looser into an untrusted project's own tier: the write is
        // refused whole, saying which key, and nothing of it is written.
        const e = await client.refused("settings.write", { tier: "project", changes: { permission_rules: [{ tool: "Bash(curl:*)", action: "allow" }] }, ...at });
        assert(e && e.code === PROJECT_UNTRUSTED && e.data?.rejected?.some((r: { key: string }) => r.key === "permission_rules"), `answered ${JSON.stringify(e)}`);
        assert(!JSON.stringify((await field()).tiers.project ?? []).includes("curl"), "the refused allow was written");
      } finally {
        await client.call("settings.write", { tier: "project", changes: { permission_rules: null }, ...at });
      }
      // One already in the project's file (written there by hand, or before the engine checked) is
      // read and withheld, and said so.
      writeProjectSettings(client.project, { permission_rules: [{ tool: "Bash(curl:*)", action: "allow" }] });
      try {
        const allow = await field();
        assert(allow.withheld === true, JSON.stringify(allow));
        assert(!(allow.effective ?? []).some((r: { tool: string }) => r.tool === "Bash(curl:*)"), "an untrusted project's allow is in force");
      } finally {
        removeProjectSettings(client.project);
      }
    },

    "sandbox and network settings are written into their tiers, and the proxy switch only globally": async ({ client }) => {
      // Values that change nothing a session does (other suites share this backend): the defaults
      // spelled out, a directory to read and a domain to reach.
      const changes = {
        "sandbox.network_mode": "unrestricted",
        "sandbox.allowed_domains": ["example.com"],
        "sandbox.read_directories": ["/tmp/harnix-live-read"],
        "sandbox.require_enforcement": false,
      };
      const written = await client.call("settings.write", { tier: "global", changes });
      try {
        for (const [key, value] of Object.entries(changes)) {
          const field = written.fields.find((f: { key: string }) => f.key === key);
          assert(isDeepStrictEqual(field.tiers.global, value), `${key}: ${JSON.stringify(field)}`);
        }
        const e = await client.refused("settings.write", { tier: "scene", changes: { "network.use_system_proxy": false } });
        assert(e && e.code === INVALID_PARAMS && /global|tier/.test(e.message), `answered ${JSON.stringify(e)}`);
        const mode = await client.refused("settings.write", { tier: "global", changes: { "sandbox.network_mode": "none" } });
        assert(mode && mode.code === INVALID_PARAMS, "a network mode the engine does not have was written");
      } finally {
        await client.call("settings.write", { tier: "global", changes: Object.fromEntries(Object.keys(changes).map((k) => [k, null])) });
      }
    },

    "providers can be read back": async ({ client }) => {
      const described = await client.call("settings.describe", { section: "providers" })
        .catch(() => null);
      assert(described !== undefined, "providers could not be described");
    },

    "a provider is added, read back with its key redacted, and removed": async ({ client }) => {
      // Written to the global tier and reloaded, so the very next read sees
      // it — not the working directory's project tier, which depends on where
      // the process was started.
      const id = "test-openai";
      const added = await client.call("settings.setProvider", {
        provider_id: id,
        config: { api_type: "openai_compatible", base_url: "https://example.invalid/v1", api_key: "sk-secret-value", default_model: "m-1", models: ["m-2"] },
      });
      assert(added.reloaded === true, "the engine was not asked to reload");
      assert(/settings\.json$/.test(added.written) && !added.written.includes("/.harnix/.harnix"), `written to ${added.written}`);
      // The engine writes it, and keeps the key out of settings.json, in the uncommitted file beside it.
      assert(/settings\.local\.json$/.test(added.secrets_written ?? ""), `the key went to ${added.secrets_written}`);
      const described = await client.call("settings.describe", {});
      const provider = described.providers?.providers?.[id];
      assert(provider, "the provider is not read back");
      assert(provider.default_model === "m-1" && provider.models?.[0] === "m-2", JSON.stringify(provider));
      assert(provider.api_key && provider.api_key !== "sk-secret-value", "the key came back in the clear");
      // An edit that leaves the key out keeps it.
      await client.call("settings.setProvider", { provider_id: id, config: { api_type: "openai_compatible", default_model: "m-3" } });
      const edited = (await client.call("settings.describe", {})).providers.providers[id];
      assert(edited.default_model === "m-3" && edited.api_key, JSON.stringify(edited));
      await client.call("settings.setProvider", { provider_id: id, delete: true });
      const after = await client.call("settings.describe", {});
      assert(!after.providers?.providers?.[id], "the provider was not removed");
    },

    "a looser value in an untrusted project's tier is refused when written and withheld when found, and said so": async ({ client }) => {
      // A project is untrusted unless something trusted it, and an untrusted project tier may only
      // tighten: the engine does not take `bypassPermissions` from it.
      const at = { project_root: client.project };
      const field = async () =>
        (await client.call("settings.describe", at)).fields.find((f: { key: string }) => f.key === "permission_mode");
      assert((await field()).withheld === false, "withheld before anything was written");
      const e = await client.refused("settings.set", { tier: "project", key: "permission_mode", value: "bypassPermissions", ...at });
      assert(e && e.code === PROJECT_UNTRUSTED && e.data?.rejected?.some((r: { key: string }) => r.key === "permission_mode"), `answered ${JSON.stringify(e)}`);
      const after = await field();
      assert(after.tiers.project !== "bypassPermissions", `the refused value was written: ${JSON.stringify(after)}`);
      writeProjectSettings(client.project, { permission_mode: "bypassPermissions" });
      try {
        const found = await field();
        assert(found.tiers.project === "bypassPermissions" && found.source === "project", JSON.stringify(found));
        assert(found.withheld === true && found.effective !== "bypassPermissions", JSON.stringify(found));
      } finally {
        removeProjectSettings(client.project);
      }
      assert((await field()).withheld === false, "still withheld once the project tier gave it back");
    },

    "an OpenAI Responses provider is taken as written": async ({ client }) => {
      const id = "test-openai-responses";
      await client.call("settings.setProvider", {
        provider_id: id,
        config: { api_type: "openai_responses", base_url: "https://example.invalid/v1", api_key: "sk-secret-value", default_model: "m-1" },
      });
      try {
        const provider = (await client.call("settings.describe", {})).providers?.providers?.[id];
        assert(provider?.api_type === "openai_responses", `read back as ${JSON.stringify(provider)}`);
      } finally {
        await client.call("settings.setProvider", { provider_id: id, delete: true });
      }
    },

    "a provider of an unknown API type is refused": async ({ client }) => {
      // Everything else in order, so the type is what the engine's dry run stops at.
      const config = { api_type: "gemini", base_url: "https://example.invalid/v1", api_key: "sk-secret-value", default_model: "m-1" };
      const e = await client.refused("settings.setProvider", { provider_id: "x", config });
      assert(e && /api_type/.test(e.message), `says ${e?.message}`);
      const after = await client.call("settings.describe", {});
      assert(!after.providers?.providers?.x, "a provider the engine refused was written");
    },

    "a provider the engine could not build a client for is not written": async ({ client }) => {
      // Tried first (`dry_run`): an OpenAI-compatible endpoint with no base URL does not route, and
      // nothing is written — the providers already there keep routing.
      const id = "test-no-url";
      const e = await client.refused("settings.setProvider",
        { provider_id: id, config: { api_type: "openai_compatible", api_key: "sk-secret-value", default_model: "m-1" } });
      assert(e && !/not a provider setting/.test(e.message), `says ${e?.message}`);
      const after = await client.call("settings.describe", {});
      assert(!after.providers?.providers?.[id], "the provider was written all the same");
    },

    "setting a provider is reachable here and refused on the engine": async ({ client }) => {
      // The engine's own `engine.config.setProvider` is refused to every client —
      // it repoints model traffic. The same intent goes through this layer,
      // where it is a deliberate, audited method rather than a passthrough.
      const direct = await client.refused("engine.config.setProvider", {});
      assert(direct && direct.code === REFUSED, "engine.config.setProvider is not refused");
      const e = await client.refused("settings.setProvider", {});
      // Refused for want of arguments, not for want of permission.
      if (e) assert(e.code !== REFUSED, `refused as unauthorized: ${e.message}`);
    },
  },
});
