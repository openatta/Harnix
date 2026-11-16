/**
 * The session space's column as drawn: the rows of the backend list, the archived ones behind
 * the toggle, and a row's `⋯` opening the shell's session menu for it.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { HostProvider, Registry, type HostApi } from "@harnix/react";

import { extension, register } from "../src/index.tsx";

afterEach(cleanup);

describe("the session space's column", () => {
  it("lists the sessions, hides the archived ones until asked, and opens the shell's menu", async () => {
    const menus: unknown[] = [];
    const host = {
      call: async (method: string) => {
        if (method === "harnix.sessions.list") return { sessions: [{ session_id: "s1", name: "one" }, { session_id: "s2", name: "two" }] };
        if (method === "harnix-app.workspaces.list") return { workspaces: [], prefs: {}, sessions: { s2: { archived: true } } };
        return {};
      },
      subscribe: () => () => undefined,
      events: { onTurnSettled: () => () => undefined },
      session: { current: () => null, context: () => null, onChange: () => () => undefined, menu: (ref: unknown) => menus.push(ref), open: () => undefined },
      shell: { notice: () => undefined },
      defaults: { instance: "main", space: "sessions" },
    } as unknown as HostApi;
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    await act(() => new Promise((resolve) => setTimeout(resolve, 0)));
    const space = r.at("space").find((s) => s.id === "sessions")!;
    const page = render(<HostProvider host={host}>{space.sidebar!.render!()}</HostProvider>);
    expect(page.getAllByTestId("session-row").map((el) => el.dataset.sid)).toEqual(["s1"]);
    fireEvent.click(page.getByTestId("archive-toggle"));
    expect(page.getAllByTestId("session-row").map((el) => el.dataset.sid).sort()).toEqual(["s1", "s2"]);
    fireEvent.click(page.getAllByTestId("session-more")[0]!);
    expect(menus).toEqual([{ instance: "main", sessionId: "s1", name: "one" }]);
  });
});

describe("importing a session bundle", () => {
  function setup(answer: () => Promise<unknown>) {
    const calls: [string, unknown][] = [];
    const opened: unknown[] = [];
    const dialogs: ((close: (r: true | null) => void) => unknown)[] = [];
    const host = {
      call: async (method: string, params: unknown) => {
        calls.push([method, params]);
        if (method === "harnix-app.sessions.importUpload") return { token: "tok", url: "/upload?token=tok" };
        if (method === "harnix-app.sessions.import") return answer();
        if (method === "harnix.sessions.list") return { sessions: [] };
        if (method === "harnix-app.workspaces.list") return { workspaces: [], prefs: {}, sessions: {} };
        return {};
      },
      subscribe: () => () => undefined,
      events: { onTurnSettled: () => () => undefined },
      session: { current: () => null, context: () => null, onChange: () => () => undefined, menu: () => undefined, open: (s: unknown) => opened.push(s) },
      dialogs: { open: async (render: (close: (r: true | null) => void) => unknown) => void dialogs.push(render) },
      shell: { notice: () => undefined },
      defaults: { instance: "main", space: "sessions" },
    } as unknown as HostApi;
    return { host, calls, opened, dialogs };
  }

  async function open(host: HostApi, dialogs: unknown[]) {
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    await act(() => new Promise((resolve) => setTimeout(resolve, 0)));
    const space = r.at("space").find((s) => s.id === "sessions")!;
    const page = render(<HostProvider host={host}>{space.sidebar!.render!()}</HostProvider>);
    fireEvent.click(page.getByTestId("session-import"));
    const body = dialogs[0] as (close: () => void) => React.ReactNode;
    return render(<HostProvider host={host}>{body(() => undefined)}</HostProvider>);
  }

  const pick = (dialog: ReturnType<typeof render>) => {
    const file = new File(["zip"], "S1.agix.zip", { type: "application/zip" });
    fireEvent.change(dialog.getByTestId("import-file"), { target: { files: [file] } });
  };

  it("uploads the file, imports it into the session space and opens the new session", async () => {
    const { host, calls, opened, dialogs } = setup(async () => ({ root_session_id: "n1", instance: "main" }));
    const sent: unknown[] = [];
    globalThis.fetch = (async (url: string, init: RequestInit) => (sent.push([url, init.method]), new Response("ok"))) as typeof fetch;
    const dialog = await open(host, dialogs);
    pick(dialog);
    await act(() => new Promise((resolve) => setTimeout(resolve, 10)));
    expect(sent).toEqual([["/upload?token=tok", "POST"]]);
    expect(calls.filter(([m]) => m.startsWith("harnix-app.sessions.import")).map(([m, p]) => [m, p])).toEqual([
      ["harnix-app.sessions.importUpload", { name: "S1.agix.zip", space: "sessions" }],
      ["harnix-app.sessions.import", { token: "tok", space: "sessions" }],
    ]);
    expect(opened).toEqual([{ instance: "main", sessionId: "n1" }]);
  });

  it("says why when the engine refuses the bundle", async () => {
    const { host, opened, dialogs } = setup(async () => {
      throw new Error("the archive is not importable");
    });
    globalThis.fetch = (async () => new Response("ok")) as typeof fetch;
    const dialog = await open(host, dialogs);
    pick(dialog);
    await act(() => new Promise((resolve) => setTimeout(resolve, 10)));
    expect(dialog.getByTestId("import-status").textContent).toContain("the archive is not importable");
    expect(opened).toEqual([]);
  });
});
