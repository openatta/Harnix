/** `SpaceContribution.tab()` (ipdrun-39): the space says which right-column tab is current. */

import { describe, expect, it } from "vitest";

import { hostFor } from "../src/host";
import { scriptedApp, space } from "./scripted";

const tabs = [
  { id: "t1", title: () => "one", render: () => "1" },
  { id: "t2", title: () => "two", render: () => "2" },
  { id: "t3", title: () => "three", render: () => "3" },
];

describe("a space's current tab", () => {
  it("is selected when the space comes to the front and at redraw; unknown ids are ignored", async () => {
    const { app } = scriptedApp();
    let current: string | undefined = "t2";
    app.registry.register("space", space("tabbed", { tabs, tab: () => current }), "test");
    const host = hostFor(app);
    const heard: string[] = [];
    host.details.onTab((tab) => heard.push(tab));

    await app.switchSpace("tabbed");
    expect(app.shownTab()).toBe("t2");
    expect(app.detailsOpen()).toBe(false); // selected, not opened

    current = "t3";
    host.redraw();
    expect(app.shownTab()).toBe("t3");

    current = "nope";
    host.redraw();
    expect(app.shownTab()).toBe("t3");
    expect(heard).toEqual(["t2", "t3"]);
  });

  it("undefined leaves the shell's choice, and a clicked tab is kept across a switch", async () => {
    const { app } = scriptedApp();
    app.registry.register("space", space("tabbed", { tabs, tab: () => undefined }), "test");
    await app.switchSpace("tabbed");
    expect(app.shownTab()).toBe("t1");
    app.selectTab("t3");
    await app.switchSpace("b");
    await app.switchSpace("tabbed");
    expect(app.shownTab()).toBe("t3");
  });

  it("a throwing tab() is the space's failure only", async () => {
    const { app } = scriptedApp();
    app.registry.register("space", space("tabbed", { tabs, tab: () => { throw new Error("tab broke"); } }), "test");
    await app.switchSpace("tabbed");
    expect(app.shownTab()).toBe("t1");
    await new Promise((r) => setTimeout(r, 0));
    expect(app.registry.refusals().map((r) => r.reason)).toContain("tab broke");
  });
});
