/**
 * Changing a session's model from where it is shown (ipdrun-12).
 *
 * `[shell] model_switch = true` opens `harnix.session.setModel` — scoped to the space the caller
 * claims, never across — and without the line nothing is added: the model is the settings'.
 */

import { INVALID_PARAMS, SPACE_MISMATCH } from "@harnix/core/protocol";
import { type Backend, connect, expect, test } from "@harnix/testkit";

import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:sessions", "@layer:api"] };

test.describe.configure({ mode: "serial" });

let backend: Backend;
test.beforeAll(async () => {
  backend = await startTestApp({ profile: "\n[shell]\nmodel_switch = true\n" });
});
test.afterAll(async () => {
  await backend?.dispose();
});

test("with model_switch, a space changes its own session's model", TAGS, async () => {
  const api = await connect(backend);
  try {
    const { methods } = await api.call("harnix.reachable");
    expect(methods).toContain("harnix.session.setModel");

    const created = await api.call("harnix.session.create", { space: "fleet" });
    const session = { instance: created.instance, session_id: created.session_id };
    // The engine's answer: the model the session's next turn runs; its context, read before that turn,
    // names it too (`fleet.model` reads `engine.session.inspectContext`).
    const chosen = await api.call("harnix.session.setModel", { ...session, space: "fleet", model: "switched-model" });
    expect(chosen).toMatchObject({ session_id: session.session_id, model: "switched-model", applies: "next_turn" });
    expect((await api.call("fleet.model", session)).model).toBe("switched-model");
  } finally {
    api.close();
  }
});

test("a space changes its own session's effort and thinking, and gives them back to the settings", TAGS, async () => {
  const api = await connect(backend);
  try {
    const created = await api.call("harnix.session.create", { space: "fleet", options: { effort: "low" } });
    const session = { instance: created.instance, session_id: created.session_id, space: "fleet" };
    // Given when it was made: the session's own.
    const made = await api.call("engine.session.get", session).catch(() => null);
    if (made?.reasoning) expect(made.reasoning).toMatchObject({ effort: "low", effort_source: "session" });
    // Changed alone: the model is left as it is.
    const changed = await api.call("harnix.session.setModel", { ...session, effort: "max", thinking_mode: "off" });
    expect(changed).toMatchObject({ session_id: session.session_id, effort: "max", thinking_mode: "off" });
    // `null` gives one back.
    const back = await api.call("harnix.session.setModel", { ...session, effort: null });
    expect(back).toMatchObject({ effort: null, thinking_mode: "off" });
    // Nothing to change is refused, and so is an effort the engine does not know.
    expect((await api.refused("harnix.session.setModel", session))?.code).toBe(INVALID_PARAMS);
    expect((await api.refused("harnix.session.setModel", { ...session, effort: "extreme" }))?.code).toBe(INVALID_PARAMS);
  } finally {
    api.close();
  }
});

test("claiming one space for another space's session is refused", TAGS, async () => {
  const api = await connect(backend);
  try {
    const created = await api.call("harnix.session.create", { space: "fleet" });
    const session = { instance: created.instance, session_id: created.session_id };
    const error = await api.refused("harnix.session.setModel", { ...session, space: "sessions", model: "x" });
    expect(error?.code).toBe(SPACE_MISMATCH);
  } finally {
    api.close();
  }
});

test("without the line, nothing is added", TAGS, async () => {
  const plain = await startTestApp();
  try {
    const api = await connect(plain);
    const { methods } = await api.call("harnix.reachable");
    expect(methods).not.toContain("harnix.session.setModel");
    api.close();
  } finally {
    await plain.dispose();
  }
});
