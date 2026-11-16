/**
 * The right column through its two doors (harnix-ext.md §22.7): `host.sidebar.*`, the navigation
 * controller over the open session's surface, and `host.details.*` / `host.reveal`, whose 0.4.4
 * semantics stay — a space's `details.tabs` and the revealed detail are the strip's other side.
 * Then the column as drawn: the guide seeded, the add control, the expand control, a close.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render, type RenderResult } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider, useTabInfo, type SidebarTabType } from "@harnix/react";

import { Shell } from "../src/Shell";
import { registerBuiltins } from "../src/builtins";
import { hostFor } from "../src/host";
import { storageKey } from "../src/sidebar/surface";
import { scriptedApp, space, tick } from "./scripted";

afterEach(cleanup);

const s1 = { instance: "main", sessionId: "s1" };
const type = (id: string, over: Partial<SidebarTabType> = {}): SidebarTabType => ({ id, kind: id, title: () => id, render: () => <div data-testid={`body-${id}`}>{id}</div>, ...over });

/** An app with the builtins (the guide), a preview type and a session open in the sessions space. */
function ready() {
  const { app } = scriptedApp();
  registerBuiltins(app);
  app.registry.register("sidebar.tab", type("preview", { priority: "builtin", patterns: ["harnix-resource://file/**"], title: (a) => (a ? a.split("/").pop()! : "Preview") }), "test");
  app.open(s1);
  return { app, host: hostFor(app) };
}

describe("host.sidebar", () => {
  it("opens a resource with the type that claims it, expands the column, focuses the session's side, and dedupes", () => {
    const { app, host } = ready();
    expect(host.sidebar.isExpanded()).toBe(false);
    const id = host.sidebar.openResource("harnix-resource://file/src/a.ts", { params: { line: 3 } });
    expect(id).toBe("tab-1");
    expect(host.sidebar.isExpanded()).toBe(true);
    expect(host.sidebar.active()).toEqual({ id, kind: "preview", address: "harnix-resource://file/src/a.ts" });
    expect(app.stripShown()).toEqual({ side: "session", tab: expect.objectContaining({ id, title: "a.ts" }) });
    expect(host.sidebar.openResource("harnix-resource://file/src/a.ts", { params: { line: 8 } })).toBe(id);
    expect(app.sidebar.tabs(s1)).toHaveLength(1);
    expect(app.sidebar.navigation(s1, id)).toEqual({ params: { line: 8 }, revision: 2 });
    // Remembered with the session.
    expect(JSON.parse(localStorage.getItem(storageKey(s1))!).layout.tabs).toEqual([id]);
    host.sidebar.toggleExpanded();
    expect(host.sidebar.isExpanded()).toBe(false);
    host.sidebar.close(id);
    expect(host.sidebar.active()).toBeNull();
    host.sidebar.close("tab-9");
  });

  it("throws for an address no type claims, a kind no type has, and with no session open", () => {
    const { app, host } = ready();
    expect(() => host.sidebar.openResource("harnix-resource://image/x.png")).toThrow(/harnix-resource:\/\/image\/x.png/);
    expect(() => host.sidebar.openResource("harnix-resource://file/x", { kind: "nope" })).toThrow(/nope/);
    expect(() => host.sidebar.openTab("nope")).toThrow(/nope/);
    app.open(null);
    expect(() => host.sidebar.openTab("guide")).toThrow();
    expect(host.sidebar.active()).toBeNull();
  });

  it("lets canOpen refuse, and opens with a named kind over the claim", () => {
    const { app, host } = ready();
    app.registry.register("sidebar.tab", type("docs", { patterns: ["*.md"], canOpen: (a) => !a.endsWith("SKIP.md") }), "ext");
    expect(host.sidebar.openResource("harnix-resource://file/README.md")).toBe("tab-1");
    expect(host.sidebar.active()?.kind).toBe("docs");
    host.sidebar.openResource("harnix-resource://file/SKIP.md");
    expect(host.sidebar.active()?.kind).toBe("preview");
    host.sidebar.openResource("harnix-resource://file/other.md", { kind: "preview" });
    expect(host.sidebar.active()?.kind).toBe("preview");
    expect(() => host.sidebar.openResource("harnix-resource://file/SKIP.md", { kind: "docs" })).toThrow(/docs/);
  });

  it("an extension's type takes a kind over the builtin's, and the builtin is back when it goes", () => {
    const { app, host } = ready();
    app.registry.activate("ext", () => {
      app.registry.register("sidebar.tab", type("ext.guide", { kind: "guide", title: () => "Ext guide" }), "ext");
    });
    host.sidebar.openTab("guide");
    const { container } = draw(app);
    expect(container.querySelector('[data-testid="body-ext.guide"]')).not.toBeNull();
    act(() => void app.registry.withdraw("ext"));
    expect(container.querySelector('[data-testid="body-ext.guide"]')).toBeNull();
    expect(container.querySelector('[data-testid="sidebar-guide"]')).not.toBeNull();
  });

  it("a scope: space type stands in the strip for the space, and openTab selects it", () => {
    const { app, host } = ready();
    app.registry.register("sidebar.tab", type("overview", { scope: "space", priority: "builtin", title: () => "Overview" }), "test");
    const seen: string[] = [];
    host.details.onTab((tab) => seen.push(tab));
    expect(host.details.tab()).toBe("overview");
    host.sidebar.openTab("guide");
    expect(app.stripShown().side).toBe("session");
    expect(host.sidebar.openTab("overview")).toBe("overview");
    expect(app.stripShown()).toEqual({ side: "space", id: "overview" });
    expect(app.sidebar.tabs(s1).map((t) => t.kind)).toEqual(["guide"]);
    const { container } = draw(app);
    expect(container.querySelector('[data-testid="body-overview"]')).not.toBeNull();
  });
});

describe("details.* and reveal keep their meaning beside the session's tabs", () => {
  it("a space's tabs are the strip's space side; details.tab() names them; a session tab opened goes beside them", async () => {
    const { app, host } = ready();
    app.registry.register("space", space("tabbed", { details: { tabs: [{ id: "t1", title: () => "one", render: () => "1" }, { id: "t2", title: () => "two", render: () => "2" }] } }), "test");
    await app.switchSpace("tabbed");
    app.open(s1, "tabbed");
    expect(host.details.tab()).toBe("t1");
    expect(app.stripShown()).toEqual({ side: "space", id: "t1" });
    host.sidebar.openTab("guide");
    expect(app.stripShown().side).toBe("session");
    // The space's selection did not move: details.tab() is what it was.
    expect(host.details.tab()).toBe("t1");
    host.details.open("t2");
    expect(host.details.tab()).toBe("t2");
    expect(app.stripShown()).toEqual({ side: "space", id: "t2" });
    // Back to the session's tab by focusing it.
    app.focusSessionTab(host.sidebar.active()!.id);
    expect(app.stripShown().side).toBe("session");
    expect(host.details.tab()).toBe("t2");
  });

  it("reveal(item) opens the details tab in front of the session's tabs, and the detail is drawn", () => {
    const { app, host } = ready();
    host.sidebar.openTab("guide");
    host.reveal({ kind: "tool", name: "Bash", id: "c1", input: "ls" });
    expect(app.stripShown()).toEqual({ side: "space", id: "details" });
    expect(host.details.tab()).toBe("details");
    expect(host.details.isOpen()).toBe(true);
    const { container } = draw(app);
    expect(container.querySelector('[data-testid="details-tool"]')).not.toBeNull();
    expect(container.querySelector('#details-tab-details[aria-selected="true"]')).not.toBeNull();
    expect(container.querySelector('[data-testid="sidebar-tab"][data-kind="guide"]')).not.toBeNull();
  });

  it("a space whose side has nothing shows the session's tab without being asked", () => {
    const { app, host } = ready();
    host.sidebar.openTab("guide");
    app.selectTab("details");
    expect(app.stripShown().side).toBe("session");
  });
});

function draw(app: ReturnType<typeof scriptedApp>["app"]): RenderResult {
  return render(
    <HarnixUiProvider client={app.conn as HarnixClient}>
      <Shell app={app} />
    </HarnixUiProvider>,
  );
}

describe("the column as drawn", () => {
  it("seeds the guide into an empty surface once the column opens with a session, and lists the guide entries", async () => {
    const { app } = ready();
    app.registry.register("sidebar.tab", type("usage", { priority: "builtin", guide: [{ id: "usage", title: () => "Usage", description: () => "Tokens spent" }] }), "test");
    app.registry.register("sidebar.tab", type("files", { priority: "builtin", guide: [{ id: "files", title: () => "Files" }] }), "test");
    const { container } = draw(app);
    expect(container.querySelector('[data-testid="sidebar-tab"]')).toBeNull();
    expect(container.querySelector('[data-testid="details-toggle"]')!.getAttribute("data-state")).toBe("collapsed");
    act(() => app.toggleDetails(true));
    await act(tick);
    expect(container.querySelector('[data-testid="details-toggle"]')!.getAttribute("data-state")).toBe("expanded");
    const guide = container.querySelector('[data-testid="sidebar-tab"][data-kind="guide"]')!;
    expect(guide.getAttribute("data-state")).toBe("current");
    expect([...container.querySelectorAll('[data-testid="sidebar-guide-entry"]')].map((e) => e.getAttribute("data-kind"))).toEqual(["usage", "files"]);
    expect(container.querySelector('[data-testid="sidebar-guide-entry"] .desc')!.textContent).toBe("Tokens spent");
    // The guide is open: no add control; choosing a card opens the page in the guide's place.
    expect(container.querySelector('[data-testid="sidebar-add"]')).toBeNull();
    fireEvent.click(container.querySelector('[data-testid="sidebar-guide-entry"][data-kind="files"]')!);
    expect([...container.querySelectorAll('[data-testid="sidebar-tab"]')].map((e) => e.getAttribute("data-kind"))).toEqual(["files"]);
    expect(container.querySelector('[data-testid="body-files"]')).not.toBeNull();
    expect(container.querySelector('[data-testid="sidebar-add"]')).not.toBeNull();
    fireEvent.click(container.querySelector('[data-testid="sidebar-add"]')!);
    expect([...container.querySelectorAll('[data-testid="sidebar-tab"]')].map((e) => e.getAttribute("data-kind"))).toEqual(["files", "guide"]);
    // Closing from the chip.
    fireEvent.click(container.querySelector('[data-testid="sidebar-tab"][data-kind="guide"] [data-testid="sidebar-tab-close"]')!);
    expect([...container.querySelectorAll('[data-testid="sidebar-tab"]')].map((e) => e.getAttribute("data-kind"))).toEqual(["files"]);
  });

  it("with exactly one guide entry, that page opens instead of the guide", async () => {
    const { app } = ready();
    app.registry.register("sidebar.tab", type("usage", { priority: "builtin", guide: [{ id: "usage", title: () => "Usage" }] }), "test");
    const { container } = draw(app);
    act(() => app.toggleDetails(true));
    await act(tick);
    expect([...container.querySelectorAll('[data-testid="sidebar-tab"]')].map((e) => e.getAttribute("data-kind"))).toEqual(["usage"]);
  });

  it("with no session the column is as it was: the details tab and its empty text", () => {
    const { app } = ready();
    app.open(null);
    app.toggleDetails(true);
    const { container } = draw(app);
    expect(container.querySelector('#details-tab-details[aria-selected="true"]')).not.toBeNull();
    expect(container.querySelector("#details .body .empty")).not.toBeNull();
    expect(container.querySelector('[data-testid="sidebar-tab"]')).toBeNull();
  });

  it("the body reads its tab domain, a keepMounted body stays hidden behind another, and the navigation reaches it", async () => {
    const { app, host } = ready();
    const Seen = () => {
      const { tab } = useTabInfo();
      return (
        <div data-testid={`seen-${tab.kind}`} data-visible={String(tab.visible)} data-revision={tab.navigation.revision}>
          {JSON.stringify(tab.navigation.params)}
        </div>
      );
    };
    app.registry.register("sidebar.tab", type("term", { priority: "builtin", keepMounted: true, render: () => <Seen /> }), "test");
    app.registry.register("sidebar.tab", type("plain", { priority: "builtin", render: () => <Seen /> }), "test");
    const { container } = draw(app);
    host.sidebar.openTab("term", { params: { cols: 80 } });
    await act(tick);
    const term = () => container.querySelector('[data-testid="seen-term"]');
    expect(term()!.getAttribute("data-visible")).toBe("true");
    expect(term()!.textContent).toBe('{"cols":80}');
    act(() => void host.sidebar.openTab("plain"));
    expect(container.querySelector('[data-testid="seen-plain"]')!.getAttribute("data-visible")).toBe("true");
    expect(term()!.getAttribute("data-visible")).toBe("false");
    expect(term()!.closest('[data-testid="sidebar-tab-body"]')!.hasAttribute("hidden")).toBe(true);
    act(() => void host.sidebar.openTab("term", { params: { cols: 120 } }));
    expect(term()!.getAttribute("data-revision")).toBe("2");
    expect(term()!.textContent).toBe('{"cols":120}');
    expect(container.querySelector('[data-testid="seen-plain"]')).toBeNull();
    // A type that has gone: its tab stays, with its stored title, and the body says why it is blank.
    act(() => void app.registry.unregister("sidebar.tab", "plain", "test"));
    act(() => app.focusSessionTab(app.sidebar.tabs(s1).find((t) => t.kind === "plain")!.id));
    expect(container.querySelector('#details .body .empty')!.textContent).toMatch(/plain/);
    expect(container.querySelector('[data-testid="sidebar-tab"][data-kind="plain"] [role="tab"]')!.textContent).toBe("plain");
  });

  it("the sessions' surfaces are their own: switching sessions switches the strip", async () => {
    const { app, host } = ready();
    host.sidebar.openTab("guide");
    const s2 = { instance: "main", sessionId: "s2" };
    app.open(s2);
    expect(host.sidebar.active()).toBeNull();
    host.sidebar.openResource("harnix-resource://file/b.ts");
    app.open(s1);
    expect(host.sidebar.active()?.kind).toBe("guide");
    // A deleted session takes its surface with it.
    app.forget(s2);
    expect(localStorage.getItem(storageKey(s2))).toBeNull();
    expect(localStorage.getItem(storageKey(s1))).not.toBeNull();
  });
});
