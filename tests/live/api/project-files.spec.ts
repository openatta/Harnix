// A session's project, read-only, for the right column (harnix-app.md §6): `harnix-app.files.list`
// and `files.read`, fenced in the session's own project root; and the changes a turn wrote, kept in
// the session's log as one `harnix.changes` entry so the changes tab still has them after a refresh.
//
// The fence is the point, as with the picker: a path is relative and climbs nowhere, and what it
// names is resolved through its links before it is read. A files tab that can be walked out of is a
// file browser for whoever can see the session.

import { mkdirSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { NOT_FOUND, REFUSED, SPACE_MISMATCH } from "@harnix/core/protocol";
import { type TestClient, assert, defineSuite, runTurn } from "@harnix/testkit";

/** A session in the test project, nothing said in it yet. */
async function inProject(client: TestClient): Promise<{ instance: string; session_id: string; space: string }> {
  const created = await client.call("harnix.session.create", { project_root: client.project, space: "sessions" });
  return { instance: created.instance, session_id: created.session_id, space: "sessions" };
}

defineSuite("project files", { feature: "session-space" }, {
  tests: {
    "a listing is one directory of the session's project, the ignored hidden until asked": async ({ client }) => {
      mkdirSync(join(client.project, "node_modules", "left-pad"), { recursive: true });
      mkdirSync(join(client.project, "src"), { recursive: true });
      writeFileSync(join(client.project, "src", "lib.rs"), "pub fn answer() -> u32 {\n    42\n}\n");
      try {
        const s = await inProject(client);
        const root = await client.call("harnix-app.files.list", s);
        assert(root.path === "", `the root's path is "${root.path}"`);
        const names = root.entries.map((e: { name: string }) => e.name);
        for (const name of ["Cargo.toml", "README.md", "src"]) assert(names.includes(name), `${name} is not listed: ${names}`);
        assert(!names.includes("node_modules"), "node_modules is listed without asking for all");
        assert(root.hidden >= 1, `hidden is ${root.hidden}`);
        // Directories first.
        assert(root.entries[0].dir === true, `the first entry is ${JSON.stringify(root.entries[0])}`);
        const all = await client.call("harnix-app.files.list", { ...s, all: true });
        assert(all.entries.some((e: { name: string }) => e.name === "node_modules"), "all: true leaves node_modules out");
        const src = await client.call("harnix-app.files.list", { ...s, path: "src" });
        assert(src.entries.length === 1 && src.entries[0].path === "src/lib.rs" && src.entries[0].size > 0, JSON.stringify(src.entries));
      } finally {
        rmSync(join(client.project, "node_modules"), { recursive: true, force: true });
        rmSync(join(client.project, "src"), { recursive: true, force: true });
      }
    },

    "a read is the file as a preview takes it": async ({ client }) => {
      const s = await inProject(client);
      const readme = await client.call("harnix-app.files.read", { ...s, path: "README.md" });
      assert(readme.kind === "text" && readme.text.includes("A project for the tests"), JSON.stringify(readme).slice(0, 200));
      writeFileSync(join(client.project, "blob.bin"), Buffer.from([0, 1, 2, 3, 0, 5]));
      try {
        const blob = await client.call("harnix-app.files.read", { ...s, path: "blob.bin" });
        assert(blob.kind === "binary" && blob.size === 6 && blob.text === undefined, JSON.stringify(blob));
      } finally {
        rmSync(join(client.project, "blob.bin"), { force: true });
      }
      const missing = await client.refused("harnix-app.files.read", { ...s, path: "no-such-file.txt" });
      assert(missing?.code === NOT_FOUND, `a missing file: ${JSON.stringify(missing)}`);
    },

    "a path out of the project is refused, by its words or by its links": async ({ client }) => {
      const s = await inProject(client);
      for (const path of ["../outside.txt", "src/../../outside.txt", "/etc/hosts", "../../../../../../etc/hosts"]) {
        const e = await client.refused("harnix-app.files.read", { ...s, path });
        assert(e?.code === REFUSED, `${path}: ${JSON.stringify(e)}`);
        const listed = await client.refused("harnix-app.files.list", { ...s, path });
        assert(listed?.code === REFUSED, `list ${path}: ${JSON.stringify(listed)}`);
      }
      // A link in the project to outside it is no way out.
      const link = join(client.project, "escape");
      symlinkSync("/etc", link);
      try {
        const through = await client.refused("harnix-app.files.read", { ...s, path: "escape/hosts" });
        assert(through?.code === REFUSED, `through a link: ${JSON.stringify(through)}`);
        const into = await client.refused("harnix-app.files.list", { ...s, path: "escape" });
        assert(into?.code === REFUSED, `listing through a link: ${JSON.stringify(into)}`);
      } finally {
        rmSync(link, { force: true });
      }
    },

    "a session is read only as the caller may see it": async ({ client }) => {
      const s = await inProject(client);
      const wrong = await client.refused("harnix-app.files.list", { ...s, space: "flow" });
      assert(wrong?.code === SPACE_MISMATCH, `claiming another space: ${JSON.stringify(wrong)}`);
      const nobody = await client.refused("harnix-app.files.list", { instance: s.instance, session_id: "no-such-session", space: "sessions" });
      assert(nobody, "a session that does not exist was listed");
    },
  },
});

defineSuite("a turn's changes", { feature: "session-space" }, {
  needsModel: true,
  replayed: true,
  tests: {
    "a turn that wrote a file leaves its changes in the session's log": async ({ client }) => {
      const created = await client.call("harnix.session.create", {
        scene: "harnix",
        project_root: client.project,
        space: "sessions",
        options: { recorder: { name: "writes-a-file" } },
      });
      const s = { instance: created.instance, session_id: created.session_id, space: "sessions" };
      await client.call("harnix.session.attach", s);
      try {
        const { settled } = await runTurn(
          client, s,
          "Use the Write tool to create notes.txt in the working directory with exactly one line: harnix-changes-fixture. Then reply: written",
        );
        assert(!settled.params.error, `the turn failed: ${JSON.stringify(settled.params.error)}`);
        // The engine said what the call wrote, beside its result.
        const results = client.eventsOfKind("tool_result", s.session_id);
        assert(results.some((r: any) => Array.isArray(r.file_changes) && r.file_changes.length === 1), "no tool_result carried file_changes");
        // And the hub kept it: one entry for the turn, pushed while the page watched.
        const pushed = await client.waitFor(
          (f) => f.method === "harnix.event" && f.params.session_id === s.session_id && f.params.event?.kind === "extension_entry"
            && f.params.event.entry?.ns === "harnix.changes",
          { describe: "the turn's changes entry to be pushed" },
        );
        const { entries } = await client.call("engine.session.entries", { ...s, namespaces: ["harnix.changes"] });
        assert(entries.length === 1 && entries[0].id === pushed.params.event.entry.id, JSON.stringify(entries));
        const payload = entries[0].payload;
        assert(entries[0].event === "turn" && payload.turn_id === settled.params.turn_id, JSON.stringify(entries[0]));
        assert(payload.files.length === 1, JSON.stringify(payload.files));
        const [file] = payload.files;
        assert(file.path.endsWith("/notes.txt") && file.op === "create" && file.added === 1 && file.removed === 0, JSON.stringify(file));
        assert(typeof file.diff === "string" && file.diff.includes("+harnix-changes-fixture"), `the diff: ${file.diff}`);
        // The file is there to preview.
        const read = await client.call("harnix-app.files.read", { ...s, path: "notes.txt" });
        assert(read.kind === "text" && read.text === "harnix-changes-fixture\n", JSON.stringify(read));
      } finally {
        rmSync(join(client.project, "notes.txt"), { force: true });
        await client.call("harnix.session.delete", s).catch(() => undefined);
      }
    },
  },
});
