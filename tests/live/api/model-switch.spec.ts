/**
 * Changing a session's model without the system space (ipdrun-12).
 *
 * A product that drops the system space drops `system.model` with it. `[shell] model_switch
 * = true` opens `harnix.session.setModel` instead — scoped to the space the caller claims, never
 * across — and without the line nothing is added.
 */

import { SPACE_MISMATCH } from "@harnix/core/protocol";
import { type Backend, connect, expect, test } from "@harnix/testkit";

import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:sessions", "@layer:api"] };

test.describe.configure({ mode: "serial" });

let backend: Backend;
test.beforeAll(async () => {
  backend = await startTestApp({ profile: "\n[shell]\nmodel_switch = true\n[extensions]\noff = [\"system\"]\n" });
});
test.afterAll(async () => {
  await backend?.dispose();
});

test("with model_switch, a space changes its own session's model", TAGS, async () => {
  const api = await connect(backend);
  try {
    const { methods } = await api.call("harnix.reachable");
    expect(methods).toContain("harnix.session.setModel");
    expect(methods).not.toContain("system.model");

    const created = await api.call("harnix.session.create", { space: "fleet" });
    const session = { instance: created.instance, session_id: created.session_id };
    // The engine's answer: the model the session's next turn runs. (Neither `engine.session.get` nor,
    // in Agix 0.3.15, `session.inspectContext` names it: the latter says the settings' model until
    // the session is rebuilt. Reported upstream.)
    const chosen = await api.call("harnix.session.setModel", { ...session, space: "fleet", model: "switched-model" });
    expect(chosen).toMatchObject({ session_id: session.session_id, model: "switched-model", applies: "next_turn" });
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
    // And it is not the way across spaces.
    const across = await api.refused("harnix.session.setModel", { ...session, space: "system", model: "x" });
    expect(across?.code).toBe(SPACE_MISMATCH);
  } finally {
    api.close();
  }
});

test("without the line, nothing is added", TAGS, async () => {
  const plain = await startTestApp({ profile: "\n[extensions]\noff = [\"system\"]\n" });
  try {
    const api = await connect(plain);
    const { methods } = await api.call("harnix.reachable");
    expect(methods).not.toContain("harnix.session.setModel");
    expect(methods).not.toContain("system.model");
    api.close();
  } finally {
    await plain.dispose();
  }
});
