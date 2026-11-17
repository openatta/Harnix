/**
 * How a failure is told to a person: the engine's `data.kind` and the host's own codes are classed;
 * a class has words, the code stays in the detail; what is not known is shown as the backend said it.
 */

import { describe, expect, it } from "vitest";
import { applySettled, emptySession, failureAction, failureClass, failureDetail, failureFacts, type SettleTexts } from "../src/state/index.ts";

const engine = (kind: string, message = "boom", extra: Record<string, unknown> = {}) => ({ code: -32002, message, data: { kind, ...extra } });

describe("failureClass", () => {
  it("the engine's kinds map to a class; the engine going away is one class", () => {
    expect(failureClass(failureFacts(engine("auth")))).toBe("auth");
    expect(failureClass(failureFacts(engine("billing")))).toBe("billing");
    expect(failureClass(failureFacts(engine("model_rejected")))).toBe("model_rejected");
    expect(failureClass(failureFacts(engine("rate_limited")))).toBe("rate_limited");
    expect(failureClass(failureFacts(engine("model_unavailable")))).toBe("model_unavailable");
    expect(failureClass(failureFacts(engine("engine_stopped")))).toBe("engine_down");
    expect(failureClass(failureFacts(engine("engine_unreachable")))).toBe("engine_down");
  });

  it("no credentials is no model, whatever kind the engine put on it", () => {
    expect(failureClass(failureFacts(engine("no_credentials", "provider `default` has no key")))).toBe("no_model");
    expect(failureClass(failureFacts(engine("model_error", "missing api_key")))).toBe("no_model");
    expect(failureClass(failureFacts(engine("auth", "missing api_key: none of X is set")))).toBe("no_model");
  });

  it("the host's own codes: an instance out of reach, an extension unavailable, a deadline", () => {
    expect(failureClass(failureFacts({ code: -31004, message: "x" }))).toBe("instance_unavailable");
    expect(failureClass(failureFacts({ code: -31010, message: "x" }))).toBe("extension_unavailable");
    expect(failureClass(failureFacts({ code: -31009, message: "x" }))).toBe("deadline");
  });

  it("what it does not know is not classed", () => {
    expect(failureClass(failureFacts(engine("internal")))).toBeUndefined();
    expect(failureClass(failureFacts(new Error("socket closed")))).toBeUndefined();
    expect(failureClass(failureFacts("plain"))).toBeUndefined();
  });

  it("the retry-after reaches the facts; the detail keeps message, code and kind", () => {
    const facts = failureFacts(engine("rate_limited", "429", { retry_after_ms: 7000 }));
    expect(facts.retryAfterMs).toBe(7000);
    expect(failureDetail(facts)).toBe("429 (-32002 rate_limited)");
  });

  it("only the model-settings problems offer the way there", () => {
    expect(failureAction("no_model")).toBe("settings.models");
    expect(failureAction("auth")).toBe("settings.models");
    expect(failureAction("model_rejected")).toBe("settings.models");
    expect(failureAction("rate_limited")).toBeUndefined();
  });
});

describe("applySettled", () => {
  const texts: SettleTexts = {
    interrupted: "interrupted",
    failed: (e) => `failed: ${e}`,
    explain: (cls) => (cls === "auth" ? "key refused" : undefined),
  };

  it("a known failure is said in words, its raw text and code in the detail, with the action", () => {
    const next = applySettled(emptySession("S1"), { error: engine("auth", "401 invalid x-api-key") }, false, texts);
    expect(next.blocks.at(-1)).toMatchObject({
      kind: "note",
      tone: "error",
      text: "failed: key refused",
      detail: "401 invalid x-api-key (-32002 auth)",
      action: "settings.models",
    });
  });

  it("one with no words, or no class, is the backend's text", () => {
    const unworded = applySettled(emptySession("S1"), { error: engine("overloaded", "busy") }, false, texts);
    expect(unworded.blocks.at(-1)).toMatchObject({ text: "failed: busy" });
    expect((unworded.blocks.at(-1) as { detail?: string }).detail).toBeUndefined();
    const unknown = applySettled(emptySession("S1"), { error: { message: "weird" } }, false, texts);
    expect(unknown.blocks.at(-1)).toMatchObject({ text: "failed: weird" });
  });

  it("an interruption is not a failure", () => {
    const next = applySettled(emptySession("S1"), { error: engine("auth") }, true, texts);
    expect(next.blocks.map((b) => (b as { text?: string }).text)).toEqual(["interrupted"]);
  });
});
