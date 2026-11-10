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
