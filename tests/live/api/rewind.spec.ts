// Going back to an earlier point of a session (`harnix.session.rewind`, harnix-app.md §5.1): the
// engine keeps a checkpoint of every file its file tools write, and the hub passes a rewind through
// to it — a dry run first, which changes nothing and tells nobody; then the rewind itself, after
// which the file is as it was, the turn is gone from the history, its changes entry is gone from
// the entries listed, and every watcher is told to read the session again.
//
// The turn is replayed from `writes-a-file`: a `Write` of `notes.txt` in the project, which did not
// exist before it — so going back to before that turn deletes the file. A change to the file made
// outside the session after the turn is a conflict: refused until it is overwritten (`force`).

import { existsSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { SPACE_MISMATCH } from "@harnix/core/protocol";
import { type TestClient, assert, connect, defineSuite, runTurn } from "@harnix/testkit";

const WRITE = "Use the Write tool to create notes.txt in the working directory with exactly one line: harnix-changes-fixture. Then reply: written";
/** The engine's: files it cannot restore as asked, nothing changed (daemon protocol §9). */
const REWIND_PRECONDITION = -32023;

type Key = { instance: string; session_id: string; space: string };

/** A session in the test project that has run the turn that writes `notes.txt`; its turn id. */
async function wrote(client: TestClient): Promise<{ s: Key; turn: string }> {
  const created = await client.call("harnix.session.create", {
    scene: "harnix",
    project_root: client.project,
    space: "sessions",
    options: { recorder: { name: "writes-a-file" } },
  });
  const s = { instance: created.instance, session_id: created.session_id, space: "sessions" };
  await client.call("harnix.session.attach", s);
  const { settled } = await runTurn(client, s, WRITE);
  assert(!settled.params.error, `the turn failed: ${JSON.stringify(settled.params.error)}`);
  assert(existsSync(join(client.project, "notes.txt")), "the turn did not write notes.txt");
  return { s, turn: settled.params.turn_id };
}

async function cleanUp(client: TestClient, s: Key | undefined): Promise<void> {
  rmSync(join(client.project, "notes.txt"), { force: true });
  if (s) await client.call("harnix.session.delete", s).catch(() => undefined);
}

defineSuite("rewind", { feature: "sessions" }, {
  tests: {
    "a dry run says what going back to before a message would do, and changes nothing": async ({ client }) => {
      let s: Key | undefined;
      try {
        const done = await wrote(client);
        s = done.s;
        const plan = await client.call("harnix.session.rewind", { ...s, to: { before_turn: done.turn }, dry_run: true });
        assert(plan.dry_run === true && plan.scope === "both", JSON.stringify(plan));
        // The first turn: before it is the start.
        assert(plan.to === "start", `to: ${JSON.stringify(plan.to)}`);
        const removed = plan.conversation?.removed_turns ?? [];
        assert(removed.length === 1 && removed[0].turn_id === done.turn && removed[0].prompt.includes("harnix-changes-fixture"), JSON.stringify(removed));
        const restored = plan.files?.restored ?? [];
        assert(restored.length === 1 && restored[0].path.endsWith("notes.txt") && restored[0].to === "absent", JSON.stringify(restored));
        assert((plan.files?.skipped ?? []).length === 0, JSON.stringify(plan.files?.skipped));
        // Nothing changed: the file, the history.
        assert(existsSync(join(client.project, "notes.txt")), "a dry run deleted the file");
        const history = await client.call("engine.session.history", { ...s, limit: 0 });
        assert(history.total > 0, `history: ${JSON.stringify(history)}`);
        assert(!client.events.some((f) => f.method === "harnix.session_resync" && f.params.session_id === s!.session_id), "a dry run resynced");
      } finally {
        await cleanUp(client, s);
      }
    },

    "a rewind puts the file back, takes the turn and its changes out, and every watcher reads the session again": async ({ client, backend }) => {
      let s: Key | undefined;
      const second = await connect(backend, { topology: client.topology });
      try {
        const done = await wrote(client);
        s = done.s;
        await second.call("harnix.session.attach", s);
        const result = await client.call("harnix.session.rewind", { ...s, to: { before_turn: done.turn } });
        assert(result.dry_run === false && result.rewind_id, JSON.stringify(result));
        assert(!existsSync(join(client.project, "notes.txt")), "notes.txt is still there");
        // Both watchers are told; the engine's own frame is not buffered for a later catch-up.
        for (const [who, c] of [["the caller", client], ["the second client", second]] as const) {
          await c.waitFor((f) => f.method === "harnix.session_resync" && f.params.session_id === s!.session_id, {
            describe: `${who} to be told to read the session again`,
          });
        }
        const history = await client.call("engine.session.history", { ...s, limit: 0 });
        assert(history.total === 0, `the history still has ${history.total} messages`);
        const attached = await second.call("harnix.session.attach", s);
        assert(attached.history_total === 0 && attached.replay.length === 0, JSON.stringify({ total: attached.history_total, replay: attached.replay.length }));
        // The turn's changes are not listed any more; the mark that says why is.
        const { entries } = await client.call("engine.session.entries", { ...s, namespaces: ["harnix.changes"] });
        assert(!entries.some((e: any) => e.event === "turn" && e.payload.turn_id === done.turn), JSON.stringify(entries));
        assert(entries.some((e: any) => e.event === "rewind" && e.payload.turn_ids.includes(done.turn)), JSON.stringify(entries));
        // The same turn is not there to go back to again.
        const again = await client.refused("harnix.session.rewind", { ...s, to: { before_turn: done.turn }, dry_run: true });
        assert(again != null, "a rewound turn was a target again");
      } finally {
        second.close();
        await cleanUp(client, s);
      }
    },

    "a file changed outside the session since is a conflict, refused until it is overwritten": async ({ client }) => {
      let s: Key | undefined;
      try {
        const done = await wrote(client);
        s = done.s;
        writeFileSync(join(client.project, "notes.txt"), "changed by hand\n");
        const plan = await client.call("harnix.session.rewind", { ...s, to: { before_turn: done.turn }, dry_run: true });
        const conflict = (plan.files?.skipped ?? []).find((i: any) => i.path.endsWith("notes.txt"));
        assert(conflict?.issue === "modified" && conflict.overridable_by === "force", JSON.stringify(plan.files));
        const refused = await client.refused("harnix.session.rewind", { ...s, to: { before_turn: done.turn } });
        assert(refused?.code === REWIND_PRECONDITION, JSON.stringify(refused));
        assert(existsSync(join(client.project, "notes.txt")), "a refused rewind changed the file");
        const forced = await client.call("harnix.session.rewind", { ...s, to: { before_turn: done.turn }, force: true });
        assert(forced.files.overwrote_external.some((p: string) => p.endsWith("notes.txt")), JSON.stringify(forced.files));
        assert(!existsSync(join(client.project, "notes.txt")), "notes.txt is still there");
      } finally {
        await cleanUp(client, s);
      }
    },

    "the files alone go back to before a tool call; the conversation stays": async ({ client }) => {
      let s: Key | undefined;
      try {
        const done = await wrote(client);
        s = done.s;
        const call = client.eventsOfKind("tool_use", s.session_id).find((e: any) => e.name === "Write");
        assert(call?.id, "no Write call was seen");
        const before = (await client.call("engine.session.history", { ...s, limit: 0 })).total;
        const result = await client.call("harnix.session.rewind", { ...s, to: { tool_use: call.id } });
        assert(result.scope === "files" && result.conversation === undefined, JSON.stringify(result));
        assert(!existsSync(join(client.project, "notes.txt")), "notes.txt is still there");
        const after = (await client.call("engine.session.history", { ...s, limit: 0 })).total;
        assert(after === before, `the history went from ${before} to ${after}`);
      } finally {
        await cleanUp(client, s);
      }
    },

    "a session of another space is not a device's to rewind": async ({ client }) => {
      const created = await client.call("harnix.session.create", { space: "sessions" });
      const s = { instance: created.instance, session_id: created.session_id, space: "sessions" };
      try {
        const spaces = (await client.call("harnix.spaces.list")).spaces.map((r: { id: string }) => r.id);
        const other = spaces.find((id: string) => id !== "sessions");
        if (!other) return;
        const refused = await client.refused("harnix.session.rewind", { ...s, space: other, to: "start", dry_run: true });
        assert(refused?.code === SPACE_MISMATCH, JSON.stringify(refused));
      } finally {
        await client.call("harnix.session.delete", s).catch(() => undefined);
      }
    },
  },
});
