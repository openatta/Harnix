/**
 * The right column's pages about the conversation's project: the files tab, the preview, the
 * changes tab, the changes card a turn leaves in the conversation, and a file tool's path opening
 * the preview at its line — through the host API and the tab domain alone.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { CHANGES_EVENT, CHANGES_NS } from "@harnix/core";
import { HostProvider, Registry, TabInfoProvider, t, type ActionTarget, type HostApi, type TabInfo } from "@harnix/react";

import { CHANGES_KIND, FILES_KIND, PREVIEW_KIND, extension, fileOfCall, lineOf, register } from "../src/index.tsx";

afterEach(cleanup);

const SESSION = { instance: "main", sessionId: "s1" };
const ROOT = "/work/alpha";

type Call = [string, Record<string, unknown>];

function fakeHost(answers: Record<string, (p: Record<string, unknown>) => unknown> = {}) {
  const calls: Call[] = [];
  const opened: unknown[][] = [];
  const host = {
    calls,
    opened,
    call: async (method: string, params: Record<string, unknown> = {}) => {
      calls.push([method, params]);
      if (method in answers) return answers[method]!(params);
      if (method === "harnix.sessions.list") return { sessions: [{ session_id: "s1", instance: "main", project_root: ROOT }] };
      if (method === "harnix-app.workspaces.list") return { workspaces: [], prefs: {}, sessions: {} };
      return {};
    },
    subscribe: () => () => undefined,
    events: { onTurnSettled: () => () => undefined, onSessionEvent: () => () => undefined, onHostEvent: () => () => undefined },
    session: { current: () => SESSION, context: () => null, onChange: () => () => undefined },
    spaces: { current: () => "projects" },
    sidebar: {
      openResource: (...args: unknown[]) => (opened.push(["resource", ...args]), "tab-1"),
      openTab: (...args: unknown[]) => (opened.push(["tab", ...args]), "tab-2"),
    },
    shell: { notice: () => undefined },
    defaults: { instance: "main", space: "sessions" },
  };
  return host as unknown as HostApi & { calls: Call[]; opened: unknown[][] };
}

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

async function registered(host: HostApi) {
  const r = new Registry();
  r.activate(extension, () => register(r, host));
  await settle();
  return r;
}

function tabInfo(kind: string, address: string | null, params: Record<string, unknown> = {}, revision = 1) {
  const actions: unknown[][] = [];
  const info: TabInfo = {
    tab: {
      id: "tab-x",
      kind,
      address,
      session: SESSION,
      visible: true,
      navigation: { params, revision },
      signal: new AbortController().signal,
      actions: {
        close: () => undefined,
        openResource: (...args) => (actions.push(["resource", ...args]), "t"),
        openTab: (...args) => (actions.push(["tab", ...args]), "t"),
        onClose: () => () => undefined,
      },
    },
  };
  return { info, actions };
}

function draw(host: HostApi, r: Registry, kind: string, info: TabInfo) {
  const type = r.at("sidebar.tab").find((x) => x.kind === kind)!;
  return render(
    <HostProvider host={host}>
      <TabInfoProvider value={info}>{type.render()}</TabInfoProvider>
    </HostProvider>,
  );
}

describe("the right column's project pages", () => {
  it("registers the files and changes pages with guide entries, and the preview as the files' fallback", async () => {
    const r = await registered(fakeHost());
    const types = r.at("sidebar.tab");
    const files = types.find((x) => x.kind === FILES_KIND)!;
    const preview = types.find((x) => x.kind === PREVIEW_KIND)!;
    const changes = types.find((x) => x.kind === CHANGES_KIND)!;
    expect([files.priority, files.patterns, files.guide?.map((g) => g.id), files.keepMounted]).toEqual(["builtin", undefined, ["files"], true]);
    expect([preview.priority, preview.patterns, preview.guide]).toEqual(["fallback", ["harnix-resource://file/**"], undefined]);
    expect(preview.title("harnix-resource://file/src/lib.rs")).toBe("lib.rs");
    expect(changes.guide?.map((g) => g.id)).toEqual(["changes"]);
    expect(types.every((x) => x.owner === "harnix-app")).toBe(true);
    const open = r.at("session.action").find((a) => a.id === "harnix-app.open-file")!;
    expect(open.at).toEqual(["arg", "tool"]);
  });

  it("finds the file a tool call names, inside the project, and where in it to look", () => {
    expect(fileOfCall({ file_path: `${ROOT}/src/a.rs`, offset: 40, limit: 20 }, ROOT)).toEqual({ path: "src/a.rs", params: { line: 40 } });
    expect(fileOfCall({ file_path: `${ROOT}/src/a.rs`, old_string: "a", new_string: "b" }, ROOT)).toEqual({ path: "src/a.rs", params: { text: "b" } });
    expect(fileOfCall({ file_path: `${ROOT}/a.rs`, edits: [{ old_string: "x", new_string: "" }, { old_string: "y", new_string: "z" }] }, ROOT)?.params).toEqual({ text: "z" });
    expect(fileOfCall({ notebook_path: `${ROOT}/n.ipynb` }, ROOT)).toEqual({ path: "n.ipynb", params: {} });
    expect(fileOfCall({ file_path: "/elsewhere/a.rs" }, ROOT)).toBeNull();
    expect(fileOfCall({ command: "ls" }, ROOT)).toBeNull();
    expect(fileOfCall({ file_path: `${ROOT}/a.rs` }, null)).toBeNull();
    expect(lineOf({ line: 7 }, null)).toBe(7);
    expect(lineOf({ text: "two" }, "one\ntwo\nthree")).toBe(2);
    expect(lineOf({ text: "four" }, "one\ntwo")).toBeNull();
  });

  it("a file tool's path opens the preview at its line, in the session's project", async () => {
    const host = fakeHost();
    const r = await registered(host);
    const open = r.at("session.action").find((a) => a.id === "harnix-app.open-file")!;
    const read: ActionTarget = { kind: "tool", session: SESSION, space: "projects", toolUseId: "u1", tool: "Read", input: { file_path: `${ROOT}/src/a.rs`, offset: 12 } };
    const bash: ActionTarget = { ...read, tool: "Bash", input: { command: "ls" } };
    const session: ActionTarget = { kind: "session", session: SESSION, space: "projects" };
    expect([open.match(read), open.match(bash), open.match(session)]).toEqual([true, false, false]);
    expect(open.label(read)).toBe(t("preview.open"));
    await open.run(read);
    expect(host.opened).toEqual([["resource", "harnix-resource://file/src/a.rs", { params: { line: 12 } }]]);
  });

  it("the files tab lists the project lazily, hides the ignored until asked, and opens a file", async () => {
    const listings: Record<string, unknown> = {
      "": { root: ROOT, path: "", entries: [{ name: "src", path: "src", dir: true }, { name: "README.md", path: "README.md", dir: false, size: 5 }], hidden: 3, truncated: false },
      src: { root: ROOT, path: "src", entries: [{ name: "a.rs", path: "src/a.rs", dir: false, size: 13 }], hidden: 0, truncated: false },
    };
    const host = fakeHost({ "harnix-app.files.list": (p) => listings[p.path as string] });
    const r = await registered(host);
    const { info, actions } = tabInfo(FILES_KIND, null);
    const page = draw(host, r, FILES_KIND, info);
    await settle();
    const rows = () => page.getAllByTestId("files-row").map((el) => el.dataset.path);
    expect(rows()).toEqual(["src", "README.md"]);
    const listed = host.calls.filter(([m]) => m === "harnix-app.files.list");
    expect(listed[0]![1]).toEqual({ instance: "main", session_id: "s1", space: "sessions", path: "", all: false });
    expect(page.getByTestId("files-toggle-ignored").textContent).toBe(t("files.showIgnored", { count: 3 }));
    fireEvent.click(page.getAllByTestId("files-row")[0]!);
    await settle();
    expect(rows()).toEqual(["src", "src/a.rs", "README.md"]);
    fireEvent.click(page.getAllByTestId("files-row")[1]!);
    expect(actions).toEqual([["resource", "harnix-resource://file/src/a.rs"]]);
    fireEvent.click(page.getByTestId("files-toggle-ignored"));
    await settle();
    expect(host.calls.filter(([m, p]) => m === "harnix-app.files.list" && p.all === true).map(([, p]) => p.path).sort()).toEqual(["", "src"]);
  });

  it("the files tab says so when the session runs in no project", async () => {
    const host = fakeHost({
      "harnix-app.files.list": () => {
        throw new Error("the session runs in no project");
      },
    });
    const r = await registered(host);
    const page = draw(host, r, FILES_KIND, tabInfo(FILES_KIND, null).info);
    await settle();
    expect(page.getByTestId("files-tab").textContent).toContain(t("files.noProject"));
  });

  it("the preview shows text with its line marked, an image, and says why it shows neither", async () => {
    const files: Record<string, unknown> = {
      "src/a.rs": { kind: "text", path: "src/a.rs", size: 20, text: "fn a() {}\nfn b() {}\nfn c() {}\n" },
      "p.png": { kind: "image", path: "p.png", size: 4, mime: "image/png", data: "iVBORw==" },
      "blob.dat": { kind: "binary", path: "blob.dat", size: 2048 },
    };
    const host = fakeHost({ "harnix-app.files.read": (p) => files[p.path as string] });
    const r = await registered(host);

    const text = draw(host, r, PREVIEW_KIND, tabInfo(PREVIEW_KIND, "harnix-resource://file/src/a.rs", { text: "fn b" }).info);
    await settle();
    expect(text.getByTestId("preview-code").dataset.lines).toBe("3");
    expect(text.getByTestId("preview-hit").dataset.line).toBe("2");
    expect(text.getByTestId("preview-tab").dataset.line).toBe("2");
    text.unmount();

    const image = draw(host, r, PREVIEW_KIND, tabInfo(PREVIEW_KIND, "harnix-resource://file/p.png").info);
    await settle();
    expect(image.getByTestId("preview-image").getAttribute("src")).toBe("data:image/png;base64,iVBORw==");
    image.unmount();

    const binary = draw(host, r, PREVIEW_KIND, tabInfo(PREVIEW_KIND, "harnix-resource://file/blob.dat").info);
    await settle();
    expect(binary.getByTestId("preview-message").textContent).toBe(t("preview.binary", { size: "2.0 KB" }));
    binary.unmount();

    const reads = host.calls.filter(([m]) => m === "harnix-app.files.read").length;
    const pdf = draw(host, r, PREVIEW_KIND, tabInfo(PREVIEW_KIND, "harnix-resource://file/doc/spec.pdf").info);
    await settle();
    expect(pdf.getByTestId("preview-message").textContent).toBe(t("preview.unsupported", { ext: "pdf" }));
    expect(host.calls.filter(([m]) => m === "harnix-app.files.read").length).toBe(reads);
  });

  it("the preview renders Markdown, and shows its source when sent to a line", async () => {
    const host = fakeHost({ "harnix-app.files.read": () => ({ kind: "text", path: "README.md", size: 9, text: "# Title\n\nbody\n" }) });
    const r = await registered(host);
    const rendered = draw(host, r, PREVIEW_KIND, tabInfo(PREVIEW_KIND, "harnix-resource://file/README.md").info);
    await settle();
    expect(rendered.getByTestId("preview-markdown").querySelector("h1")?.textContent).toBe("Title");
    fireEvent.click(rendered.getByTestId("preview-toggle-source"));
    expect(rendered.getByTestId("preview-code")).toBeTruthy();
    rendered.unmount();
    const atLine = draw(host, r, PREVIEW_KIND, tabInfo(PREVIEW_KIND, "harnix-resource://file/README.md", { line: 3 }).info);
    await settle();
    expect(atLine.getByTestId("preview-hit").dataset.line).toBe("3");
  });
});

describe("a turn's changes", () => {
  const entry = (turn: string, files: unknown[]) => ({
    id: `e-${turn}`,
    ts: new Date().toISOString(),
    ns: CHANGES_NS,
    event: CHANGES_EVENT,
    at: 4,
    payload: { turn_id: turn, files, added: 3, removed: 1 },
  });
  const files = [
    { path: `${ROOT}/src/a.rs`, op: "modify", added: 2, removed: 1, diff: "--- a/src/a.rs\n+++ b/src/a.rs\n@@ -10,2 +10,3 @@\n keep\n-old\n+new\n+more\n" },
    { path: `${ROOT}/notes.bin`, op: "create", added: 1, removed: 0, omitted: "binary" },
  ];

  it("leaves a card in the conversation that opens the changes tab at its turn", async () => {
    const host = fakeHost();
    const r = await registered(host);
    const block = { key: 1, kind: "custom" as const, type: "entry", data: entry("t7", files) };
    const card = r.claim("flow.block", block)!;
    expect(card.id).toBe("harnix-app.changes");
    expect(r.claim("flow.block", { ...block, data: { ...block.data, ns: "harnix.audit" } })).toBeNull();
    const page = render(<HostProvider host={host}>{card.render(block)}</HostProvider>);
    expect(page.getByTestId("changes-card").dataset.turn).toBe("t7");
    expect(page.getByTestId("changes-card").textContent).toContain(t("changes.card", { count: 2 }));
    expect(page.getByTestId("changes-card").textContent).toContain("a.rs, notes.bin");
    fireEvent.click(page.getByTestId("changes-card-open"));
    expect(host.opened).toEqual([["tab", CHANGES_KIND, { params: { turn: "t7" } }]]);
  });

  it("the changes tab lists a turn's files from the session's log, each with its diff", async () => {
    const host = fakeHost({ "engine.session.entries": () => ({ entries: [entry("t1", files.slice(0, 1)), entry("t2", files)] }) });
    const r = await registered(host);
    const { info, actions } = tabInfo(CHANGES_KIND, null, { turn: "t2" });
    const page = draw(host, r, CHANGES_KIND, info);
    await settle();
    expect(host.calls.find(([m]) => m === "engine.session.entries")![1]).toEqual({ instance: "main", session_id: "s1", space: "sessions", namespaces: [CHANGES_NS] });
    expect(page.getAllByTestId("changes-turn").map((el) => el.dataset.turn)).toEqual(["t2"]);
    expect(page.getAllByTestId("changes-file").map((el) => el.dataset.path)).toEqual(["src/a.rs", "notes.bin"]);
    const lines = page.getAllByTestId("conv-diff-line").map((el) => `${el.dataset.op}:${el.textContent}`);
    expect(lines).toEqual(["same:1010keep", "del:11−old", "add:11+new", "add:12+more"]);
    expect(page.getAllByTestId("changes-file")[1]!.textContent).toContain(t("changes.omitted.binary"));
    fireEvent.click(page.getAllByTestId("changes-open-file")[0]!);
    expect(actions).toEqual([["resource", "harnix-resource://file/src/a.rs"]]);
    fireEvent.click(page.getByTestId("changes-show-all"));
    expect(page.getAllByTestId("changes-turn").map((el) => el.dataset.turn)).toEqual(["t2", "t1"]);
  });
});
