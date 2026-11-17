import { describe, expect, it } from "vitest";
import { answerOf, askForm, elicitationContent, elicitationFields, type AskForm } from "../src/state/asks.ts";
import { answerAsk, applyEvent, emptySession, reopenAsk, type BlockOf, type SessionState } from "../src/state/model.ts";

const opts = { promptTimeoutSecs: 300, now: 1_000 };

function run(events: Record<string, unknown>[], state: SessionState = { ...emptySession("S"), loading: false }) {
  return events.reduce((s, e) => applyEvent(s, e as never, opts), state);
}
const asks = (s: SessionState) => s.blocks.filter((b): b is BlockOf<"ask"> => b.kind === "ask");

describe("提问读成表单", () => {
  it("权限照旧；模型的提问带问句、短标题和选项", () => {
    expect(askForm({ kind: "prompt", prompt_id: "p", tool_name: "Bash" }, [])).toEqual({ type: "permission" });
    const q = askForm(
      { kind: "prompt", prompt_type: "question", prompt_id: "u1", tool_use_id: "u1", question: "Which branch?", header: "Branch", options: [{ key: "a", label: "main" }, { key: "b", label: "dev" }] },
      [],
    );
    expect(q).toEqual({ type: "question", question: "Which branch?", header: "Branch", options: [{ key: "a", label: "main" }, { key: "b", label: "dev" }] });
    const free = askForm({ kind: "prompt", prompt_type: "question", prompt_id: "u2", question: "Name?", header: null, options: [] }, []);
    expect(free).toMatchObject({ type: "question", header: null, options: [] });
  });

  it("离开计划模式是计划审阅：计划是那次调用之前的回复，带它要的放行", () => {
    const s = run([
      { kind: "user_message", text: "plan it" },
      { kind: "text_delta", text: "## Plan\n\n1. add `a.txt`\n2. test" },
      { kind: "tool_use", id: "x1", name: "ExitPlanMode", input: { allowed_prompts: [{ tool: "Bash", prompt: "run tests" }, { tool: "Bash" }] } },
      { kind: "prompt", prompt_type: "permission", prompt_id: "p1", tool_name: "ExitPlanMode", tool_use_id: "x1", message: "Exit plan mode?" },
    ]);
    expect(asks(s)[0]!.form).toEqual({ type: "plan", plan: "## Plan\n\n1. add `a.txt`\n2. test", allowed: [{ tool: "Bash", prompt: "run tests" }] });
    // 这一轮还没说话：计划为空，不去拿上一轮的。
    const bare = run([{ kind: "text_delta", text: "old" }, { kind: "user_message", text: "again" }, { kind: "prompt", prompt_id: "p2", tool_name: "ExitPlanMode" }]);
    expect(asks(bare)[0]!.form).toEqual({ type: "plan", plan: "", allowed: [] });
  });

  it("MCP server 的询问：表单的字段按 schema 读，网址的不带字段", () => {
    const form = askForm(
      {
        kind: "prompt", prompt_type: "mcp_elicitation", prompt_id: "e1", server: "github", mode: "form", message: "Pick a repo",
        requested_schema: {
          type: "object",
          properties: {
            repo: { type: "string", title: "Repository", minLength: 1 },
            count: { type: "integer", minimum: 1, maximum: 5, default: 2 },
            ratio: { type: "number" },
            private: { type: "boolean", description: "Only private ones" },
            color: { type: "string", enum: ["r", "g"], enumNames: ["Red", "Green"] },
            size: { oneOf: [{ const: "s", title: "Small" }, { const: "l", title: "Large" }] },
            tags: { type: "array", items: { type: "string" } },
          },
          required: ["repo", "color"],
        },
      },
      [],
    );
    expect(form.type).toBe("elicitation");
    const fields = (form as Extract<AskForm, { type: "elicitation" }>).fields;
    expect(fields.map((f) => [f.name, f.kind, f.required])).toEqual([
      ["repo", "text", true],
      ["count", "integer", false],
      ["ratio", "number", false],
      ["private", "boolean", false],
      ["color", "choice", true],
      ["size", "choice", false],
      ["tags", "unsupported", false],
    ]);
    expect(fields[0]).toMatchObject({ title: "Repository", minLength: 1 });
    expect(fields[1]).toMatchObject({ minimum: 1, maximum: 5, default: 2 });
    expect(fields[4]!.choices).toEqual([{ key: "r", label: "Red" }, { key: "g", label: "Green" }]);
    expect(fields[5]!.choices).toEqual([{ key: "s", label: "Small" }, { key: "l", label: "Large" }]);

    const page = askForm({ kind: "prompt", prompt_type: "mcp_elicitation", prompt_id: "e2", server: "auth", mode: "url", message: "Sign in", url: "https://auth.example.com/x?y=1", url_host: "auth.example.com" }, []);
    expect(page).toEqual({ type: "elicitation", server: "auth", message: "Sign in", limited: false, fields: [], url: { href: "https://auth.example.com/x?y=1", host: "auth.example.com" } });
  });

  it("填的东西读成 content；交不了的说哪个字段、为什么", () => {
    const fields = elicitationFields({
      type: "object",
      properties: {
        name: { type: "string", maxLength: 3 },
        n: { type: "integer", minimum: 1, maximum: 5 },
        x: { type: "number" },
        ok: { type: "boolean" },
        pick: { type: "string", enum: ["a", "b"] },
        note: { type: "string" },
      },
      required: ["name", "pick"],
    });
    expect(elicitationContent(fields, { name: "bob", n: "3", x: "1.5", ok: true, pick: "b" })).toEqual({
      content: { name: "bob", n: 3, x: 1.5, ok: true, pick: "b" },
    });
    expect(elicitationContent(fields, { name: "", n: "2.5", x: "abc", pick: "" })).toEqual({
      problems: { name: "required", n: "integer", x: "number", pick: "required" },
    });
    expect(elicitationContent(fields, { name: "long", n: "9", pick: "a" })).toEqual({ problems: { name: "length", n: "range" } });
    const unsupported = elicitationFields({ type: "object", properties: { tags: { type: "array" } }, required: ["tags"] });
    expect(elicitationContent(unsupported, {})).toEqual({ problems: { tags: "unsupported" } });
  });
});

describe("回答在卡片上", () => {
  const question: AskForm = { type: "question", question: "?", header: null, options: [{ key: "a", label: "main" }] };
  it("每种回答各有结局，提问的答案说选项的名字", () => {
    expect(answerOf({ type: "permission" }, { decision: { type: "permit_always", scope: "session" } })).toEqual({ answered: "allow" });
    expect(answerOf({ type: "permission" }, { decision: { type: "deny", reason: "user denied" } })).toEqual({ answered: "deny" });
    expect(answerOf({ type: "plan", plan: "", allowed: [] }, { decision: { type: "deny", reason: "use b.txt" } })).toEqual({ answered: "deny", reply: "use b.txt" });
    expect(answerOf(question, { answer: { type: "answer", text: "a" } })).toEqual({ answered: "replied", reply: "main" });
    expect(answerOf(question, { answer: { type: "answer", text: "my own" } })).toEqual({ answered: "replied", reply: "my own" });
    expect(answerOf(question, { answer: { type: "decline" } })).toEqual({ answered: "declined" });
    expect(answerOf({ type: "elicitation", server: "s", message: "", limited: false, fields: [] }, { answer: { type: "accept" } })).toEqual({ answered: "accepted" });
    expect(answerOf({ type: "elicitation", server: "s", message: "", limited: false, fields: [] }, { answer: { type: "cancel" } })).toEqual({ answered: "cancelled" });
  });

  it("中枢说的答案画到别处的卡片上，不管它先到还是引擎的 prompt_resolved 先到；本页答的不动", () => {
    const asked = { kind: "prompt", prompt_type: "question", prompt_id: "q1", question: "?", options: [{ key: "a", label: "main" }] };
    const told = { kind: "ask_answered", prompt_id: "q1", prompt_type: "question", answer: { type: "answer", text: "a" } };
    const resolved = { kind: "prompt_resolved", prompt_id: "q1", resolution: "answered" };
    for (const order of [[told, resolved], [resolved, told]]) {
      const s = run([asked, ...order]);
      expect(asks(s)[0]).toMatchObject({ answered: "replied", reply: "main", elsewhere: true });
    }
    const here = run([asked]);
    const mine = answerAsk(here, asks(here)[0]!.key, "declined");
    expect(asks(run([told], mine))[0]).toMatchObject({ answered: "declined" });
    expect(asks(run([told], mine))[0]!.elsewhere).toBeUndefined();
  });

  it("没被收下的回答让卡片回到没答；别处答的不回去", () => {
    const s = run([{ kind: "prompt", prompt_type: "mcp_elicitation", prompt_id: "e1", server: "s", mode: "form", requested_schema: {} }]);
    const key = asks(s)[0]!.key;
    const answered = answerAsk(s, key, "accepted");
    expect(asks(reopenAsk(answered, key))[0]!.answered).toBeUndefined();
    const elsewhere = run([{ kind: "prompt_resolved", prompt_id: "e1", resolution: "answered" }], s);
    expect(asks(reopenAsk(elsewhere, key))[0]!.answered).toBe("answered");
  });
});
