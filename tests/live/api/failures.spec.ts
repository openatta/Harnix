/**
 * How a failed turn arrives, which is what the interface has words for (`classOf` below is the
 * interface's own classing, `failureClass` in `@harnix/core/state`): the engine says why in
 * `data.kind` of the turn's error, and a person is told what happened and what to do next
 * (harnix-app.md, "错误呈现"). What is checked here is the shape the interface reads — that each
 * common failure arrives classed, with the engine's own words and code kept — not the words.
 *
 * The model service is a stub that fails on purpose (`stub-provider.ts`), so no real service, key or
 * recording is involved, and the backend is not replaying.
 *
 * The engine retries what is worth retrying (a rate limit, an overload, a dropped connection)
 * before it gives up, so those turns take their time: the timeouts here are long on purpose.
 */

import { failureClass, failureFacts, type FailureClass } from "@harnix/core/state";
import { type Backend, type TestClient, connect, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { FAULTS, dropStub, stubProvider, useStub, type Fault, type StubProvider } from "./stub-provider";

const TAGS = { tag: ["@feature:sessions", "@layer:api"] };

let backend: Backend;
let api: TestClient;
let stub: StubProvider;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  // No key in the environment: until a provider is set there is no model at all.
  backend = await startHarnix(launchFor("none", { env: { ANTHROPIC_API_KEY: "", ANTHROPIC_AUTH_TOKEN: "" } }));
  stub = await stubProvider();
});
test.afterAll(async () => {
  await stub?.close();
  await backend?.dispose();
});
test.beforeEach(async () => {
  api = await connect(backend);
});
test.afterEach(() => api?.close());

/** What a message to a fresh session comes to: the refusal of the create or the send, or the turn's own error. */
async function failureOfATurn(): Promise<unknown> {
  const created = await api.call("harnix.session.create", {}).catch((e: unknown) => ({ refused: e }));
  if ("refused" in created) return created.refused;
  const s = { instance: created.instance, session_id: created.session_id };
  await api.call("harnix.session.attach", s);
  const sent = await api.call("harnix.session.send", { ...s, message: "hello" }).catch((e: unknown) => ({ refused: e }));
  if ("refused" in sent) return sent.refused;
  const settled = await api.waitFor(
    (f) => f.method === "harnix.turn_settled" && f.params.session_id === s.session_id,
    { timeout: 120_000, describe: "the failing turn to settle" },
  );
  expect(settled.params.error, "the turn did not fail").toBeTruthy();
  return settled.params.error;
}

function classOf(error: unknown): FailureClass | undefined {
  return failureClass(failureFacts(error));
}

test("no provider and no key: a message is refused as no model, not as a bare code", TAGS, async () => {
  const error = await failureOfATurn();
  expect(classOf(error), JSON.stringify(error)).toBe("no_model");
  // The engine's own words stay, for whoever supports it.
  expect(failureFacts(error).raw).not.toBe("");
});

for (const [name, fault, expected] of [
  ["a key the service rejects (401)", FAULTS.auth, "auth"],
  ["a key that may not use the model (403)", FAULTS.forbidden, "auth"],
  ["a model name the service does not know (404)", FAULTS.wrongModel, "model_rejected"],
  ["an account out of credit", FAULTS.billing, "billing"],
] as [string, Fault, FailureClass][]) {
  test(`${name} arrives as ${expected}, with the engine's words and kind`, TAGS, async () => {
    stub.fail(fault);
    await useStub(api, stub);
    try {
      const error = await failureOfATurn();
      expect(classOf(error), JSON.stringify(error)).toBe(expected);
      expect((error as { data?: { kind?: string } }).data?.kind).toBeTruthy();
    } finally {
      await dropStub(api);
    }
  });
}

test("a rate limit arrives as rate_limited, with how long the service asked to wait when the engine says", TAGS, async () => {
  stub.fail(FAULTS.rateLimited);
  await useStub(api, stub);
  try {
    const error = await failureOfATurn();
    expect(classOf(error), JSON.stringify(error)).toBe("rate_limited");
    // `retry_after_ms` is what the interface turns into "wait N s"; absent, it says "a moment".
    const wait = failureFacts(error).retryAfterMs;
    if (wait !== undefined) expect(wait).toBeGreaterThan(0);
  } finally {
    await dropStub(api);
  }
});

test("a service that cannot be reached arrives as model_unavailable (a timeout is the same class)", TAGS, async () => {
  stub.fail(FAULTS.drop);
  await useStub(api, stub);
  try {
    const error = await failureOfATurn();
    expect(classOf(error), JSON.stringify(error)).toBe("model_unavailable");
    expect(stub.requests(), "the engine tried the service").toBeGreaterThan(0);
  } finally {
    await dropStub(api);
  }
});
