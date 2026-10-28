// The ceilings, approached rather than read.
//
// Every number in `harnix.hello`'s `limits` used to be checked the same way:
// `> 0`. That says a field is present, not that the number means anything —
// and two of the three meant nothing at all. `max_frame_bytes` was published
// and enforced nowhere; `max_upload_bytes` was published as 32 MB while the
// web framework's own untouched default refused everything over 2 MB with a
// sentence naming neither Harnix nor the channel. Installing an extension
// package of any real size was broken, and no test could see it because no
// test ever sent anything big.
//
// So these are slow and allocate. That is the cost of testing a limit: you
// have to reach it.

import { BUSY, CONTRIB_API_VERSION, INVALID_PARAMS, PROTOCOL_VERSION } from "@harnix/core/protocol";
import { defineSuite, finish } from "@harnix/testkit";

import { startTestApp } from "./testapp";

const assert = (cond, message) => { if (!cond) throw new Error(message); };

defineSuite("limits", { feature: "connection" }, {
  tests: {
    "hello reports every ceiling a client has to respect": async ({ client }) => {
      const { limits } = await client.call("harnix.hello");
      for (const key of ["max_frame_bytes", "max_upload_bytes", "replay_max_frames", "max_in_flight"]) {
        assert(typeof limits[key] === "number" && limits[key] > 0,
          `limits.${key} is ${JSON.stringify(limits[key])}`);
      }
      // The three were assembled by hand here once, and `max_upload_bytes`
      // was simply left out — a client had no way to learn the number it
      // would be held to.
      assert(Object.keys(limits).length === 4,
        `limits carries ${Object.keys(limits).join(", ")}`);
    },

    "a frame over the ceiling is refused, and the connection survives":
      async ({ client }) => {
        const { limits } = await client.call("harnix.hello");
        // Just over. Building the whole ceiling plus one is 16 MB of string
        // for a check that triggers at exactly the same place.
        const message = "x".repeat(limits.max_frame_bytes + 1024);
        const e = await client.refused("harnix.send",
          { session_id: "no-such-session", message });
        assert(e, "a frame over the ceiling was accepted");
        assert(String(e.message).includes(String(limits.max_frame_bytes)),
          `the refusal does not name the ceiling: "${e.message}"`);

        // Still usable. Dropping the connection would leave the client
        // guessing which frame did it, and reconnecting to find out.
        const hello = await client.call("harnix.hello");
        assert(hello.protocol_version, "the connection did not survive the refusal");
      },

    "a frame just under the ceiling goes through": async ({ client }) => {
      // The other half of a limit: it has to let the legal case past. A
      // ceiling enforced one byte low is indistinguishable from one enforced
      // correctly until somebody sends the largest legal thing.
      //
      // Aimed at a session that does not exist, deliberately. What is under
      // test is that the frame *arrives* — and "session not found" is proof
      // it reached dispatch, without spending a turn to find out.
      const { limits } = await client.call("harnix.hello");
      const message = "x".repeat(Math.floor(limits.max_frame_bytes * 0.9));
      const e = await client.refused("harnix.send",
        { session_id: "no-such-session", message });
      assert(e, "a legal frame was answered as if the session existed");
      assert(!String(e.message).includes("ceiling"),
        `a legal frame was refused for its size: "${e.message}"`);
      assert(/not found|no such/i.test(e.message), `answered "${e.message}"`);
    },

    "an upload is held to the ceiling its own grant was issued under":
      async ({ backend, client }) => {
        const { limits } = await client.call("harnix.hello");
        const grant = await client.call("harnix.upload.begin",
          { name: "big.bin", bytes: 1 });
        assert(grant.max_bytes === limits.max_upload_bytes,
          `the grant says ${grant.max_bytes}, hello says ${limits.max_upload_bytes}`);

        const over = await fetch(`http://127.0.0.1:${backend.port}${grant.url}`, {
          method: "POST",
          body: Buffer.alloc(grant.max_bytes + 1024),
        });
        assert(over.status === 413, `an oversized upload answered ${over.status}`);
        const said = await over.text();
        // Harnix's own sentence, naming the number. The framework's default
        // said "failed to buffer the request body", which tells a caller
        // neither what the limit is nor that there is a bulk channel at all.
        assert(said.includes(String(grant.max_bytes)),
          `the refusal does not name the ceiling: "${said}"`);
      },

    "a package may be larger than a file, because its grant says so":
      async ({ backend, client }) => {
        // The two things that use this channel have different ceilings, and
        // the ceiling travels with the grant rather than being one number for
        // the route. A package upload at a size a file upload would refuse is
        // the case that proves it.
        const file = await client.call("harnix.upload.begin", { name: "f.bin", bytes: 1 });
        const pkg = await client.call("harnix.plugins.upload", { name: "p.zip" });
        assert(pkg.max_bytes > file.max_bytes,
          `package ceiling ${pkg.max_bytes} is not above the file ceiling ${file.max_bytes}`);

        const body = Buffer.alloc(file.max_bytes + 4 * 1024 * 1024);
        const accepted = await fetch(`http://127.0.0.1:${backend.port}${pkg.url}`,
          { method: "POST", body });
        assert(accepted.ok,
          `a package inside its own ceiling was refused: ${accepted.status} ${await accepted.text()}`);
      },

    "asking for a grant above the ceiling is refused before one exists":
      async ({ client }) => {
        const { limits } = await client.call("harnix.hello");
        const e = await client.refused("harnix.upload.begin",
          { name: "huge.bin", bytes: limits.max_upload_bytes + 1 });
        assert(e && e.code === INVALID_PARAMS, `code ${e?.code}`);
        assert(e.message.includes(String(limits.max_upload_bytes)),
          `does not name the ceiling: "${e.message}"`);
      },

    "more requests in flight than the cap are answered, not dropped":
      async ({ client }) => {
        // The transport holds a semaphore of `max_in_flight`. What a client must never see is a
        // request that disappears: every one of them comes back, as an answer or as `BUSY`.
        const n = 40;
        const answers = await Promise.allSettled(
          Array.from({ length: n }, () => client.call("daemon.ping")));
        assert(answers.length === n, `${answers.length} of ${n} came back`);
        for (const a of answers) {
          if (a.status === "fulfilled") assert(a.value.pong === true, `an answer was ${JSON.stringify(a.value)}`);
          else assert(a.reason?.code === BUSY, `a refusal was ${a.reason?.message}`);
        }
      },

    "a request over the in-flight cap is answered BUSY, on a raw connection": async () => {
      // Raw, so no client-side queue stands between the test and the cap (ipdrun-01): forty slow
      // calls at once on one connection. Every one is answered — slept, or refused with the number.
      const backend = await startTestApp();
      try {
        const ws = new WebSocket(`ws://127.0.0.1:${backend.port}/ws?token=${encodeURIComponent(backend.token)}`);
        await new Promise((r) => { ws.onopen = r; });
        const answers = new Map();
        const settled = new Map();
        ws.onmessage = (e) => {
          const frame = JSON.parse(e.data);
          if (frame.id !== undefined) { answers.set(frame.id, frame); settled.get(frame.id)?.(); }
        };
        const send = (id, method, params) => new Promise((resolve) => {
          settled.set(id, resolve);
          ws.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
        });
        await send(0, "harnix.handshake",
          { protocol_version: PROTOCOL_VERSION, contrib_api_version: CONTRIB_API_VERSION, topology: "single_duplex" });
        const { result: hello } = await send(1, "harnix.hello", {}).then(() => answers.get(1));
        assert(hello.limits.max_in_flight === 16, `max_in_flight is ${hello.limits.max_in_flight}`);

        const n = 40;
        const ids = Array.from({ length: n }, (_, i) => 100 + i);
        const all = Promise.all(ids.map((id) => send(id, "app.probe.sleep", { ms: 400 })));
        const timeout = new Promise((_, reject) => setTimeout(() => reject(new Error(
          `${n - [...answers.keys()].filter((id) => id >= 100).length} of ${n} never answered`)), 10_000));
        await Promise.race([all, timeout]);
        ws.close();

        let slept = 0, busy = 0;
        for (const id of ids) {
          const frame = answers.get(id);
          if (frame.result) { slept += 1; continue; }
          assert(frame.error.code === BUSY, `answered ${JSON.stringify(frame.error)}`);
          assert(frame.error.data?.max_in_flight === hello.limits.max_in_flight,
            `BUSY carries ${JSON.stringify(frame.error.data)}`);
          busy += 1;
        }
        assert(slept >= 1 && slept <= hello.limits.max_in_flight, `${slept} ran`);
        assert(busy === n - slept, `${busy} refused, ${slept} ran`);
      } finally {
        backend.stop();
      }
    },

    "the replay buffer is bounded and says when it truncated":
      async ({ client }) => {
        // Not filled here — 20 000 frames is a live turn nobody wants in a
        // suite. What is checked is that the field a client branches on
        // exists and is a boolean on a fresh attach, because the interface
        // shows a banner off it and a missing field reads as "not truncated".
        const created = await client.call("session.create", {});
        const attached = await client.call("harnix.attach", { session_id: created.session_id });
        assert(typeof attached.truncated === "boolean",
          `truncated is ${JSON.stringify(attached.truncated)}`);
        assert(attached.truncated === false, "a fresh session reports a truncated buffer");
        assert(Array.isArray(attached.replay), "replay is not an array");
        await finish(client, created.session_id);
      },
  },
});
