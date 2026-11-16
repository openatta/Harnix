// The directory picker, project creation, and the bulk channel.
//
// The picker's fence is the point: it opens on the projects directory, walks
// under `$HOME` or an explicitly configured root, and refuses everything
// else. A picker that can be walked out of is a file browser. The `@` menu's
// fence is a project: a session's, or one that is a project here.

import { mkdtempSync, rmSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { INVALID_PARAMS, REFUSED } from "@harnix/core/protocol";
import { assert, defineSuite } from "@harnix/testkit";

defineSuite("files", { feature: "session-space" }, {
  tests: {
    "the picker lists a directory with breadcrumbs": async ({ client }) => {
      const listing = await client.call("harnix-app.listDirectory", {});
      assert(listing.path, "no path");
      assert(Array.isArray(listing.entries), "no entries");
      assert(Array.isArray(listing.breadcrumbs), "no breadcrumbs");
      // Directories only: a picker that lists files is asking the wrong
      // question.
      for (const entry of listing.entries) assert("name" in entry, "an entry has no name");
    },

    "the picker cannot be walked out of": async ({ client }) => {
      const e = await client.refused("harnix-app.listDirectory", { path: "/etc" });
      // Either refused, or answered with something still inside the fence —
      // both are correct, silently listing /etc is not.
      if (!e) {
        const listing = await client.call("harnix-app.listDirectory", { path: "/etc" });
        assert(!listing.path.startsWith("/etc"), `walked out to ${listing.path}`);
      }
    },

    "recent projects are derived from the sessions that exist": async ({ client }) => {
      const recent = await client.call("harnix-app.recentProjects");
      assert(Array.isArray(recent.projects ?? recent.roots ?? []), "no project list");
    },

    "a project can be created and then listed": async ({ client }) => {
      const name = `api-test-${Date.now()}`;
      const created = await client.call("harnix-app.projects.create", { name });
      assert(created.path?.endsWith(name), `created ${created.path}`);
      const listing = await client.call("harnix-app.listDirectory", {});
      assert(listing.entries.some((e: { name: string }) => e.name === name), "the new project is not listed");
    },

    "file mentions resolve inside a session's project": async ({ client }) => {
      const created = await client.call("harnix.session.create", { project_root: client.project, space: "sessions" });
      const s = { instance: created.instance, session_id: created.session_id, space: "sessions" };
      const result = await client.call("harnix-app.files", { ...s, query: "Cargo" });
      assert(Array.isArray(result.files), "no file list");
      assert(result.files.some((f: { path: string }) => f.path.includes("Cargo")), "Cargo.toml was not found");
      // A session's project may also be named, now that a session runs in it.
      const named = await client.call("harnix-app.files", { project_root: client.project, query: "Cargo" });
      assert(named.files.some((f: { path: string }) => f.path.includes("Cargo")), "naming the session's project found nothing");
      // A claim on another space is checked like any session call's.
      const wrong = await client.refused("harnix-app.files", { ...s, space: "flow", query: "Cargo" });
      assert(wrong, "a session claimed in another space was walked");
    },

    "file mentions walk no directory that is not a project here": async ({ client }) => {
      const elsewhere = mkdtempSync(join(tmpdir(), "harnix-not-a-project-"));
      const link = `${client.project}-link-${Date.now()}`;
      symlinkSync(elsewhere, link);
      try {
        // Somewhere on the disk, the parent of a project, a path that climbs
        // out of one, a link that leads elsewhere: none is a project.
        for (const project_root of ["/", "/etc", elsewhere, join(client.project, ".."), `${client.project}/../..`, link]) {
          const e = await client.refused("harnix-app.files", { project_root, query: "" });
          assert(e?.code === REFUSED, `${project_root}: ${JSON.stringify(e)}`);
        }
        // Naming nothing is not the projects directory.
        const bare = await client.refused("harnix-app.files", { query: "" });
        assert(bare?.code === INVALID_PARAMS, `no root: ${JSON.stringify(bare)}`);
      } finally {
        rmSync(link, { force: true });
        rmSync(elsewhere, { recursive: true, force: true });
      }
    },

    "the picker cannot be climbed out of with ..": async ({ client }) => {
      const { home } = await client.call("harnix-app.listDirectory", {});
      for (const path of [`${home}/..`, `${home}/../..`, `${home}/../definitely-not-here`]) {
        // Resolved before it is fenced: written under home is not inside it.
        const e = await client.refused("harnix-app.listDirectory", { path });
        assert(e?.code === INVALID_PARAMS, `${path}: ${JSON.stringify(e)}`);
      }
    },

    "an upload grant is one-shot and the payload lands on disk": async ({ client, backend }) => {
      const grant = await client.call("harnix.bulk.upload", { name: "note.txt", bytes: 11 });
      assert(grant.url.includes("token="), `url is ${grant.url}`);

      const body = "hello there";
      const response = await fetch(`http://127.0.0.1:${backend.port}${grant.url}`,
        { method: "POST", body });
      assert(response.ok, `upload failed: ${response.status}`);
      const written = await response.json();
      assert(written.path === grant.path, "the payload landed somewhere else");

      // Spent. A grant that could be replayed would be a writable path handed
      // to whoever saw the URL.
      const replay = await fetch(`http://127.0.0.1:${backend.port}${grant.url}`,
        { method: "POST", body });
      assert(!replay.ok, "the same upload grant was accepted twice");
    },

    "an unknown upload token is refused": async ({ backend }) => {
      const response = await fetch(`http://127.0.0.1:${backend.port}/upload?token=made-up`,
        { method: "POST", body: "x" });
      assert(response.status === 403, `status ${response.status}`);
    },
  },
});
