// Say the demo's conversations to the Harnix app running under --replay-dir (xops-04 §8).
//
//   node scripts/demo/talk.mjs <url> <token file> <conversations.json>
//
// Each conversation's session is the one the flow module gives it — a flow's own conversation,
// the general one, a run's main session — so it replays the recording named after it
// (scripts/demo/recordings.py made them from the same script). A run's turn with `task` says the
// station's task message as the dispatcher would: its protocol (with the rules), then its skill.
// The questions a turn raises are answered as its `prompts` say, in order (`permit` when none
// is left).
import { readFileSync } from "node:fs";

import { connect } from "../../ui/packages/testkit/src/api.ts";

const [url, tokenFile, scriptFile] = process.argv.slice(2);
if (!url || !tokenFile || !scriptFile) {
  console.error("usage: talk.mjs <url> <token file> <conversations.json>");
  process.exit(2);
}
const backend = { url: url.replace(/\/$/, ""), token: readFileSync(tokenFile, "utf8").trim() };
const { conversations } = JSON.parse(readFileSync(scriptFile, "utf8"));
const api = await connect(backend);

/** A SKILL.md without its front matter. */
function skillBody(text) {
  const t = text.trimStart();
  if (t.startsWith("---\n")) {
    const end = t.indexOf("\n---", 4);
    if (end >= 0) return t.slice(end + 4).trim();
  }
  return t.trim();
}

/** What the dispatcher would send a station's task. */
async function taskMessage(run, station) {
  const { record } = await api.call("app.flow.get", { run });
  const def = await api.call("app.flow.def", { name: record.flow });
  const protocol = def.protocol[station];
  if (!protocol) throw new Error(`${run}: station ${station} has no session task`);
  let text = protocol.replaceAll("<run>", run).replaceAll("<subject>", record.subject);
  const skill = def.def.stations.find((s) => s.id === station)?.skill;
  const body = skill && def.parts?.skills?.[skill];
  if (body) text += `\n\n## Skill \`${skill}\`\n\n${skillBody(body)}\n`;
  return text;
}

async function sessionOf(conv) {
  if (conv.kind === "general") return api.call("app.flow.session", {});
  if (conv.kind === "flow") return api.call("app.flow.session", { flow: conv.flow });
  const { main } = await api.call("app.flow.sessions", { run: conv.run });
  if (!main) throw new Error(`${conv.run} has no main session`);
  return main;
}

/** One turn, its questions answered as scripted; resolves once it settles. */
async function turn(session, message, prompts) {
  const answers = [...(prompts ?? [])];
  const seen = new Set();
  const sent = await api.call("harnix.send", { session_id: session.session_id, message });
  for (;;) {
    const f = await api.waitFor(
      (f) =>
        (f.method === "harnix.turn_settled" && f.params.turn_id === sent.turn_id) ||
        (f.method === "harnix.event" &&
          f.params.session_id === session.session_id &&
          f.params.event?.kind === "prompt" &&
          !seen.has(f.params.event.prompt_id)),
      { timeout: 120_000, describe: "the turn to settle, or a question" },
    );
    if (f.method === "harnix.turn_settled") return f.params;
    seen.add(f.params.event.prompt_id);
    const a = answers.shift() ?? { decision: "permit" };
    const decision = a.decision === "deny" ? { type: "deny", reason: a.reason ?? "" } : { type: "permit" };
    await api.call("session.respondToPrompt", {
      session_id: session.session_id,
      prompt_id: f.params.event.prompt_id,
      decision,
    });
  }
}

let failed = 0;
for (const conv of conversations) {
  const label = conv.run ?? conv.flow ?? "general";
  try {
    const session = await sessionOf(conv);
    await api.call("harnix.attach", { session_id: session.session_id });
    for (const t of conv.turns) {
      const message = t.task ? await taskMessage(conv.run, t.task) : t.say;
      const settled = await turn(session, message, t.prompts);
      if (settled.outcome?.status && settled.outcome.status !== "ok") {
        throw new Error(`a turn ended ${settled.outcome.status}: ${JSON.stringify(settled.outcome).slice(0, 300)}`);
      }
    }
    console.log(`  ${label}: ${conv.turns.length} turn(s)`);
  } catch (e) {
    failed += 1;
    console.error(`  ${label}: ${e.message ?? e}`);
  }
}
api.close();
process.exit(failed ? 1 : 0);
