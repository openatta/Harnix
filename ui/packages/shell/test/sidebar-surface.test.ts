/**
 * A session's docking surface (harnix-ext.md §22.7): what is stored and read back, what is dropped
 * as invalid, how opening deduplicates, how closing hands over, and what a close handler may keep.
 */

import { beforeEach, describe, expect, it } from "vitest";

import { STORAGE_PREFIX, SurfaceStore, emptySurface, storageKey, validateSurface, type SurfaceState } from "../src/sidebar/surface";

const s1 = { instance: "main", sessionId: "s1" };
const s2 = { instance: "main", sessionId: "s2" };

function stored(session = s1): SurfaceState | null {
  const raw = localStorage.getItem(storageKey(session));
  return raw === null ? null : (JSON.parse(raw) as SurfaceState);
}

beforeEach(() => localStorage.clear());

describe("validation", () => {
  const good: SurfaceState = {
    version: 1,
    layout: { type: "pane", id: "p1", tabs: ["tab-1", "tab-2"], active: "tab-2" },
    tabs: [
      { id: "tab-1", kind: "guide", address: null, title: "Guide" },
      { id: "tab-2", kind: "preview", address: "harnix-resource://file/a.md", title: "a.md" },
    ],
    nextId: 3,
  };

  it("takes a well-formed surface as it is, and the empty one", () => {
    expect(validateSurface(good)).toEqual(good);
    expect(validateSurface(JSON.parse(JSON.stringify(emptySurface())))).toEqual(emptySurface());
  });

  it("refuses the wrong version, a pane naming a tab that is not there, a tab in no pane, a stray selection, a counter behind the ids", () => {
    const over = (change: (s: SurfaceState) => void) => {
      const copy = JSON.parse(JSON.stringify(good)) as SurfaceState;
      change(copy);
      return validateSurface(copy);
    };
    expect(validateSurface(null)).toBeNull();
    expect(validateSurface("x")).toBeNull();
    expect(over((s) => ((s as { version: number }).version = 2))).toBeNull();
    expect(over((s) => s.layout.tabs.push("tab-9"))).toBeNull();
    expect(over((s) => s.layout.tabs.pop())).toBeNull();
    expect(over((s) => (s.layout.active = "tab-9"))).toBeNull();
    expect(over((s) => (s.layout.active = null))).toBeNull();
    expect(over((s) => (s.nextId = 2))).toBeNull();
    expect(over((s) => (s.nextId = 2.5))).toBeNull();
    expect(over((s) => ((s.tabs[0] as { address: unknown }).address = 7))).toBeNull();
    expect(over((s) => ((s.tabs[0] as { kind: unknown }).kind = ""))).toBeNull();
    expect(over((s) => ((s.layout as { type: string }).type = "split"))).toBeNull();
    expect(over((s) => s.tabs.push({ ...s.tabs[0]! }))).toBeNull();
  });
});

describe("the store", () => {
  it("reads a session's surface from storage, and drops that session's key alone when it is invalid", () => {
    localStorage.setItem(storageKey(s1), JSON.stringify({ version: 1, layout: { type: "pane", id: "p1", tabs: ["nope"], active: "nope" }, tabs: [], nextId: 1 }));
    localStorage.setItem(storageKey(s2), JSON.stringify({ ...emptySurface(), nextId: 4 }));
    localStorage.setItem("harnix.other", "kept");
    const store = new SurfaceStore(localStorage);
    expect(store.surface(s1)).toEqual(emptySurface());
    expect(localStorage.getItem(storageKey(s1))).toBeNull();
    expect(store.surface(s2).nextId).toBe(4);
    expect(localStorage.getItem("harnix.other")).toBe("kept");
    expect(storageKey(s1)).toBe(`${STORAGE_PREFIX}main/s1`);
    // Not JSON at all: dropped too.
    localStorage.setItem(storageKey({ instance: "main", sessionId: "s3" }), "{not json");
    expect(store.surface({ instance: "main", sessionId: "s3" })).toEqual(emptySurface());
    expect(localStorage.getItem(storageKey({ instance: "main", sessionId: "s3" }))).toBeNull();
  });

  it("stores every change under the session's key, and keeps sessions apart", () => {
    const store = new SurfaceStore(localStorage);
    const a = store.open(s1, { kind: "guide", address: null, title: "Guide" });
    const b = store.open(s1, { kind: "preview", address: "harnix-resource://file/a.md", title: "a.md" });
    store.open(s2, { kind: "guide", address: null, title: "Guide" });
    expect(stored()!.layout).toEqual({ type: "pane", id: "p1", tabs: [a, b], active: b });
    expect(stored()!.nextId).toBe(3);
    expect(stored(s2)!.layout.tabs).toHaveLength(1);
    // Read back by a fresh store, as a reload would.
    const again = new SurfaceStore(localStorage);
    expect(again.tabs(s1).map((t) => t.id)).toEqual([a, b]);
    expect(again.active(s1)?.id).toBe(b);
    // The next id goes on from where it was: an id never comes back.
    expect(again.open(s1, { kind: "x", address: null, title: "x" })).toBe("tab-3");
  });

  it("lives in memory when storage fails, and when there is none", () => {
    const broken = {
      getItem: () => {
        throw new Error("blocked");
      },
      setItem: () => {
        throw new Error("blocked");
      },
      removeItem: () => {
        throw new Error("blocked");
      },
    };
    const store = new SurfaceStore(broken);
    const id = store.open(s1, { kind: "guide", address: null, title: "Guide" });
    expect(store.active(s1)?.id).toBe(id);
    const none = new SurfaceStore(null);
    expect(none.open(s1, { kind: "guide", address: null, title: "Guide" })).toBe("tab-1");
    expect(none.tabs(s1)).toHaveLength(1);
  });

  it("focuses the same content rather than opening it twice, unless told to or the type is multiple", () => {
    const store = new SurfaceStore(localStorage);
    const a = store.open(s1, { kind: "preview", address: "harnix-resource://file/a.md", title: "a", params: { line: 1 } });
    const guide = store.open(s1, { kind: "guide", address: null, title: "Guide" });
    expect(store.active(s1)?.id).toBe(guide);
    expect(store.open(s1, { kind: "preview", address: "harnix-resource://file/a.md", title: "a", params: { line: 9 } })).toBe(a);
    expect(store.active(s1)?.id).toBe(a);
    expect(store.navigation(s1, a)).toEqual({ params: { line: 9 }, revision: 2 });
    expect(store.tabs(s1)).toHaveLength(2);
    // Another page of the same kind is the same page: focused, not opened.
    expect(store.open(s1, { kind: "guide", address: null, title: "Guide" })).toBe(guide);
    // Told not to reveal: a tab of its own, beside the first.
    const twin = store.open(s1, { kind: "preview", address: "harnix-resource://file/a.md", title: "a", revealIfOpened: false });
    expect(twin).not.toBe(a);
    expect(store.tabs(s1)).toHaveLength(3);
    // A multiple type: every opening is its own tab, with an address of its own.
    const o1 = store.open(s1, { kind: "output", address: null, title: "out", multiple: true });
    const o2 = store.open(s1, { kind: "output", address: null, title: "out", multiple: true });
    expect(o1).not.toBe(o2);
    expect(store.tab(s1, o1)?.address).toMatch(/^harnix-tab:\/\/output\//);
    expect(store.tab(s1, o1)?.address).not.toBe(store.tab(s1, o2)?.address);
  });

  it("puts a replacing tab where the replaced one was, and closes that one", () => {
    const store = new SurfaceStore(localStorage);
    const a = store.open(s1, { kind: "a", address: null, title: "a" });
    const guide = store.open(s1, { kind: "guide", address: null, title: "Guide" });
    const c = store.open(s1, { kind: "c", address: null, title: "c" });
    const signal = store.signal(s1, guide);
    const opened = store.open(s1, { kind: "files", address: null, title: "Files", replaceTab: guide });
    expect(store.tabs(s1).map((t) => t.id)).toEqual([a, opened, c]);
    expect(store.active(s1)?.id).toBe(opened);
    expect(signal.aborted).toBe(true);
    // Replacing a tab that is already gone: the new one goes at the end.
    const d = store.open(s1, { kind: "d", address: null, title: "d", replaceTab: "tab-99" });
    expect(store.tabs(s1).map((t) => t.id)).toEqual([a, opened, c, d]);
  });

  it("closing hands over to the left neighbour, runs the close handlers first, and a throw keeps the tab", () => {
    const store = new SurfaceStore(localStorage);
    const failures: string[] = [];
    store.failed = (tabId, e) => failures.push(`${tabId}: ${(e as Error).message}`);
    const a = store.open(s1, { kind: "a", address: null, title: "a" });
    const b = store.open(s1, { kind: "b", address: null, title: "b" });
    const c = store.open(s1, { kind: "c", address: null, title: "c" });
    store.select(s1, b);
    const ran: string[] = [];
    const off = store.onClose(s1, b, () => ran.push("b"));
    store.onClose(s1, c, () => {
      throw new Error("still saving");
    });
    expect(store.close(s1, c)).toBe(false);
    expect(store.tabs(s1)).toHaveLength(3);
    expect(failures).toEqual([`${c}: still saving`]);
    expect(store.close(s1, b)).toBe(true);
    expect(ran).toEqual(["b"]);
    expect(store.active(s1)?.id).toBe(a);
    expect(store.signal(s1, a).aborted).toBe(false);
    off();
    expect(store.close(s1, "tab-77")).toBe(false);
    // The one in front at the left end: its right neighbour takes over.
    store.select(s1, a);
    store.close(s1, a);
    expect(store.active(s1)?.id).toBe(c);
    expect(stored()!.layout.tabs).toEqual([c]);
  });

  it("drops a session: storage, surface, and the signals of its tabs", () => {
    const store = new SurfaceStore(localStorage);
    const id = store.open(s1, { kind: "a", address: null, title: "a" });
    const signal = store.signal(s1, id);
    store.drop(s1);
    expect(signal.aborted).toBe(true);
    expect(localStorage.getItem(storageKey(s1))).toBeNull();
    expect(store.loaded()).toEqual([]);
    expect(store.tabs(s1)).toEqual([]);
  });

  it("tells its subscribers of each change, and remembers which bodies have been in front", () => {
    const store = new SurfaceStore(localStorage);
    let told = 0;
    store.subscribe(() => (told += 1));
    const id = store.open(s1, { kind: "a", address: null, title: "a" });
    expect(told).toBe(1);
    expect(store.visited(s1, id)).toBe(false);
    store.visit(s1, id);
    store.visit(s1, id);
    expect(store.visited(s1, id)).toBe(true);
    expect(told).toBe(2);
  });
});
