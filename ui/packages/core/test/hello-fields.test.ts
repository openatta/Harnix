import { describe, expect, it } from "vitest";
import type { Hello } from "../src/protocol/index.ts";

// `Hello` is generated from the struct the server serializes (`harnix_contract::wire::Hello`), so a
// field read under a name the server never sends is a type error here, not a countdown that is
// always 300 s. These pin the two names that once drifted.

describe("hello 的字段名与服务端一致", () => {
  it("权限询问的超时是 hello 顶层的 permission_prompt_timeout_secs", () => {
    const hello = { permission_prompt_timeout_secs: 2 } as Pick<Hello, "permission_prompt_timeout_secs">;
    expect(hello.permission_prompt_timeout_secs).toBe(2);
  });

  it("limits 里没有 prompt_timeout_secs", () => {
    // @ts-expect-error `limits` carries no prompt timeout
    const limits: Hello["limits"] = { max_frame_bytes: 1, max_upload_bytes: 1, replay_max_frames: 1, max_in_flight: 1, prompt_timeout_secs: 1 };
    expect(limits).toBeDefined();
  });
});
