import { describe, expect, it } from "vitest";
import type { Hello } from "../src/protocol/index.ts";
import serverSource from "../../../../crates/assembly/src/kernel_rows/hello.rs?raw";

// The server's `harnix.hello` is assembled by hand in Rust; the names the client reads must be the
// ones it sends.

describe("hello 的字段名与服务端一致", () => {
  it("权限询问的超时是 hello 顶层的 permission_prompt_timeout_secs", () => {
    const hello = { permission_prompt_timeout_secs: 2 } as Hello;
    expect(hello.permission_prompt_timeout_secs).toBe(2);
    expect(serverSource).toContain('"permission_prompt_timeout_secs": self.prompt_timeout_secs');
  });

  it("limits 里没有 prompt_timeout_secs", () => {
    expect(serverSource).not.toContain('"prompt_timeout_secs"');
    // @ts-expect-error `limits` carries no prompt timeout
    const limits: NonNullable<Hello["limits"]> = { prompt_timeout_secs: 1 };
    expect(limits).toBeDefined();
  });
});
