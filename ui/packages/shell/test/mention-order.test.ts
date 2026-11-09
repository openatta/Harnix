/** `SpaceContribution.mention.first` (ipdrun-37): whose `@` candidates come first. */

import { describe, expect, it } from "vitest";

import { withContributed } from "../src/commands";
import { scriptedApp, space } from "./scripted";

const files = Array.from({ length: 25 }, (_, i) => ({ id: `f${i}.md`, insert: `@f${i}.md ` }));

describe("the @ menu", () => {
  it("lists the shell's own first by default, and the contributed first when the space says so", async () => {
    const { app } = scriptedApp();
    app.registry.register("command", { id: "items", trigger: "@", complete: () => [{ replacement: "@IT-1 ", label: "IT-1" }] }, "test");
    app.registry.register("space", space("listed", { mention: { first: "contributed" } }), "test");

    const plain = await withContributed(app, "", files);
    expect(plain).toHaveLength(20);
    expect(plain.map((m) => m.id)).not.toContain("IT-1");

    app.registry.register("space", space("files-first", { mention: { first: "files" } }), "test");
    await app.switchSpace("files-first");
    expect((await withContributed(app, "", files))[0]!.id).toBe("f0.md");

    await app.switchSpace("listed");
    const first = await withContributed(app, "", files);
    expect(first).toHaveLength(20);
    expect(first[0]).toEqual({ id: "IT-1", label: undefined, insert: "@IT-1 " });
    expect(first[1]!.id).toBe("f0.md");
  });
});
