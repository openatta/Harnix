/** `host.composer.insert(text, {session})` (ipdrun-36): into that session's composer, now or when it is on the page. */

import { describe, expect, it } from "vitest";
import type { ComposerApi } from "@harnix/react";

import { keyOf } from "../src/app";
import { hostFor } from "../src/host";
import { scriptedApp } from "./scripted";

function composer(): ComposerApi & { got: string[] } {
  const got: string[] = [];
  return { got, insert: (text) => got.push(text) };
}

describe("composer.insert", () => {
  it("without a session goes to whichever composer is on the page, as before", () => {
    const { app } = scriptedApp();
    const host = hostFor(app);
    host.composer.insert("lost"); // none on the page: nothing, as before
    const page = composer();
    app.composer = page;
    host.composer.insert("hello");
    expect(page.got).toEqual(["hello"]);
  });

  it("with a session: at once if its composer is on the page, else kept and put in when it is", () => {
    const { app } = scriptedApp();
    const host = hostFor(app);
    const a = { instance: "system", sessionId: "a" };
    const b = { instance: "system", sessionId: "b" };
    const onA = composer();
    app.composer = onA;
    app.composerBound(keyOf(a.instance, a.sessionId), onA);

    // Before `open(b)`, and after: neither lands in a's composer.
    host.composer.insert("one ", { session: b });
    host.session.open(b);
    host.composer.insert("two", { session: b });
    expect(onA.got).toEqual([]);

    // a's conversation leaves, b's comes: what was kept goes in, once.
    app.composerGone(keyOf(a.instance, a.sessionId), onA);
    const onB = composer();
    app.composerBound(keyOf(b.instance, b.sessionId), onB);
    expect(onB.got).toEqual(["one two"]);
    host.composer.insert("three", { session: b });
    expect(onB.got).toEqual(["one two", "three"]);

    // Bound again later (the conversation drawn anew): nothing is kept twice.
    const again = composer();
    app.composerBound(keyOf(b.instance, b.sessionId), again);
    expect(again.got).toEqual([]);

    // A composer that left only unbinds itself.
    app.composerGone(keyOf(b.instance, b.sessionId), onB);
    host.composer.insert("four", { session: b });
    expect(again.got).toEqual(["four"]);
  });
});
