/** Settings · MCP servers: listed with what the engine says, added, edited with values kept, removed after asking, signed in to. */

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { HarnixClient, McpList, McpServer } from "@harnix/core";
import { HarnixUiProvider, Registry, en, type HostApi } from "@harnix/react";

import { extension, givenOf, mcpState, register, signIn } from "../src/index.tsx";

afterEach(cleanup);

const client = { status: "closed", detail: undefined, onStatus: () => () => undefined, call: async () => ({}) } as unknown as HarnixClient;

function server(over: Partial<McpServer>): McpServer {
  return { name: "files", tier: "global", enabled: true, transport: "stdio", command: "fs-server", args: [], env: {}, headers: {}, unsupported: [], ...over };
}

const status = (state: string, auth = "none", extra: Record<string, unknown> = {}) => ({ state, tool_count: 2, auth: { state: auth, scopes: [] }, ...extra });

function mount(list: McpList, answers: Record<string, unknown> = {}, confirm = false) {
  const calls: [string, Record<string, unknown>][] = [];
  const notices: string[] = [];
  const host = {
    call: async (method: string, params: Record<string, unknown>) => {
      calls.push([method, params]);
      return method in answers ? answers[method] : list;
    },
    session: { current: () => null, context: () => null },
    shell: { notice: (text: string) => notices.push(text) },
    dialogs: { confirm: async () => confirm },
  } as unknown as HostApi;
  const r = new Registry();
  r.activate(extension, () => register(r, host));
  const mcp = r.at("settings.section").find((s) => s.id === "mcp")!;
  render(<HarnixUiProvider client={client} strings={en}>{mcp.render()}</HarnixUiProvider>);
  return { calls, notices };
}

const paths = { global: "/g/settings.json", scene: null, project: "/p/.harnix/settings.json" };

describe("Settings · MCP", () => {
  it("says what each server is doing, and what it asks for that this build lacks", async () => {
    const list: McpList = {
      servers: [
        server({ name: "ok", status: status("connected") }),
        server({ name: "broken", status: status("failed", "none", { error: "no such file" }) }),
        server({ name: "docs", transport: "http", url: "https://docs.example/mcp", command: undefined, status: status("auth_required", "required"), unsupported: ["private_key_jwt"] }),
        server({ name: "resting", enabled: false }),
        server({ name: "ok", tier: "project", overridden_by: undefined, status: status("connected") }),
      ],
      paths,
      status_read: true,
    };
    mount(list);
    const card = async (name: string, tier = "global") => (await screen.findAllByText(name)).map((n) => n.closest("[data-mcp]")!).find((c) => c.getAttribute("data-tier") === tier)!;
    expect((await card("ok")).getAttribute("data-state")).toBe("connected");
    expect((await card("broken")).textContent).toContain("no such file");
    const docs = await card("docs");
    expect(docs.getAttribute("data-state")).toBe("auth_required");
    expect(docs.querySelector('[data-testid="mcp-signin"]')).toBeTruthy();
    expect(docs.querySelector('[data-testid="mcp-jwt"]')).toBeTruthy();
    const resting = await card("resting");
    expect(resting.getAttribute("data-state")).toBe("off");
    expect(resting.querySelector('[data-testid="mcp-toggle"]')!.textContent).toBe("Switch on");
    expect(resting.querySelector('[data-testid="mcp-edit"]')).toBeNull();
  });

  it("reads a server's state the way the row draws it", () => {
    expect(mcpState(server({ enabled: false, status: status("connected") })).key).toBe("off");
    expect(mcpState(server({})).key).toBe("unknown");
    expect(mcpState(server({ status: status("connected", "authorized") }))).toEqual({ key: "authorized", tone: "ok" });
    expect(mcpState(server({ status: status("connected", "required") })).key).toBe("auth_required");
    expect(mcpState(server({ status: status("auth_required", "failed") })).key).toBe("signInFailed");
    expect(mcpState(server({ status: status("untrusted") }))).toEqual({ key: "untrusted", tone: "err" });
  });

  it("adds a server into the tier chosen, its values as given", async () => {
    const { calls } = mount({ servers: [], paths, status_read: true });
    fireEvent.click(await screen.findByTestId("mcp-add"));
    fireEvent.change(screen.getByTestId("mcp-tier"), { target: { value: "project" } });
    fireEvent.change(screen.getByTestId("mcp-name"), { target: { value: "fs" } });
    fireEvent.change(screen.getByTestId("mcp-command"), { target: { value: "npx" } });
    fireEvent.change(screen.getByTestId("mcp-args"), { target: { value: "-y\n@x/server\n" } });
    fireEvent.click(screen.getByTestId("mcp-env-add"));
    fireEvent.change(screen.getByTestId("mcp-env-name"), { target: { value: "TOKEN" } });
    // In a project's tier a value cannot come from the engine's environment.
    expect([...screen.getByTestId("mcp-env-mode").querySelectorAll("option")].map((o) => o.value)).toEqual(["secret"]);
    fireEvent.change(screen.getByTestId("mcp-env-value"), { target: { value: "s3cret" } });
    expect(screen.getByTestId("mcp-env-value").getAttribute("type")).toBe("password");
    fireEvent.click(screen.getByTestId("mcp-save"));
    await waitFor(() => expect(calls.some(([m]) => m === "settings.mcp.add")).toBe(true));
    const [, params] = calls.find(([m]) => m === "settings.mcp.add")!;
    expect(params).toMatchObject({ tier: "project", name: "fs", config: { transport: "stdio", command: "npx", args: ["-y", "@x/server"], env: { TOKEN: { secret: "s3cret" } } } });
  });

  it("edits a server keeping the values it never saw, and a remote one by its URL and headers", async () => {
    const list: McpList = {
      servers: [
        server({ name: "gh", transport: "http", command: undefined, url: "https://gh.example/mcp", headers: { Authorization: { kind: "secret", masked: "***abcd" }, "X-User": { kind: "env", var: "GH_USER" } } }),
      ],
      paths,
      status_read: true,
    };
    const { calls } = mount(list);
    fireEvent.click(await screen.findByTestId("mcp-edit"));
    expect(screen.queryByTestId("mcp-name")).toBeNull();
    const modes = screen.getAllByTestId("mcp-headers-mode") as HTMLSelectElement[];
    expect(modes.map((m) => m.value)).toEqual(["keep", "var"]);
    expect(screen.getByText("Keep ***abcd")).toBeTruthy();
    fireEvent.change(screen.getByTestId("mcp-url"), { target: { value: "https://gh.example/v2/mcp" } });
    fireEvent.click(screen.getByTestId("mcp-save"));
    await waitFor(() => expect(calls.some(([m]) => m === "settings.mcp.update")).toBe(true));
    const [, params] = calls.find(([m]) => m === "settings.mcp.update")!;
    expect(params).toMatchObject({ tier: "global", name: "gh", config: { transport: "http", url: "https://gh.example/v2/mcp", headers: { Authorization: { keep: true }, "X-User": { var: "GH_USER" } } } });
    expect(JSON.stringify(params)).not.toContain("***abcd");
  });

  it("removes a server only once it is confirmed", async () => {
    const list: McpList = { servers: [server({})], paths, status_read: true };
    const no = mount(list, {}, false);
    fireEvent.click(await screen.findByTestId("mcp-remove"));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(no.calls.some(([m]) => m === "settings.mcp.remove")).toBe(false);
    cleanup();
    const yes = mount(list, {}, true);
    fireEvent.click(await screen.findByTestId("mcp-remove"));
    await waitFor(() => expect(yes.calls.some(([m]) => m === "settings.mcp.remove")).toBe(true));
    expect(yes.calls.find(([m]) => m === "settings.mcp.remove")![1]).toMatchObject({ tier: "global", name: "files" });
  });

  it("switches a server off and on, and reconnects it, in its own tier", async () => {
    const { calls } = mount({ servers: [server({ tier: "project", status: status("connected") })], paths, status_read: true });
    fireEvent.click(await screen.findByTestId("mcp-toggle"));
    fireEvent.click(screen.getByTestId("mcp-reconnect"));
    await waitFor(() => expect(calls.filter(([m]) => m !== "settings.mcp.list").length).toBe(2));
    expect(calls.find(([m]) => m === "settings.mcp.enable")![1]).toMatchObject({ tier: "project", name: "files", enabled: false });
    expect(calls.find(([m]) => m === "settings.mcp.reconnect")![1]).toMatchObject({ tier: "project", name: "files" });
  });

  it("turns the form's rows into what the extension takes", () => {
    expect(
      givenOf([
        { name: " A ", mode: "secret", value: "v" },
        { name: "B", mode: "var", value: " HOME " },
        { name: "C", mode: "keep", value: "" },
        { name: "", mode: "secret", value: "dropped" },
      ]),
    ).toEqual({ A: { secret: "v" }, B: { var: "HOME" }, C: { keep: true } });
  });
});

describe("signing in", () => {
  /** A window whose `open` hands back a popup this test moves about. */
  function browser() {
    const popup = { closed: false, opener: {} as unknown, location: { href: "about:blank" } as { href: string }, close() { this.closed = true; } };
    let tick: (() => void) | null = null;
    const win = {
      location: { origin: "http://127.0.0.1:4317" },
      open: () => popup,
      setInterval: (fn: () => void) => {
        tick = fn;
        return 1;
      },
      clearInterval: () => {
        tick = null;
      },
    } as unknown as Window;
    return { popup, win, tick: () => tick?.() };
  }

  function host(answers: Record<string, unknown>) {
    const calls: [string, Record<string, unknown>][] = [];
    const notices: [string, string | undefined][] = [];
    const h = {
      call: async (method: string, params: Record<string, unknown>) => {
        calls.push([method, params]);
        const answer = answers[method];
        if (answer instanceof Error) throw answer;
        return answer;
      },
      shell: { notice: (text: string, tone?: string) => notices.push([text, tone]) },
      t: (key: string, vars?: Record<string, unknown>) => `${key}${vars ? JSON.stringify(vars) : ""}`,
    } as unknown as HostApi;
    return { h, calls, notices };
  }

  it("sends the person to the authorization page and hands the way back over once it is here", async () => {
    const { popup, win, tick } = browser();
    const started = { flow_id: "F", authorize_url: "https://as.example/authorize?state=S", redirect: "external", callback: "/ext/settings/mcp/callback", expires_in: 600 };
    const { h, calls, notices } = host({ "settings.mcp.authorize": started, "settings.mcp.authComplete": { server: "docs", ok: true, scopes: ["a"] } });
    let settled = 0;
    const done = signIn(h, "docs", { scene: "code", project_root: null }, () => settled++, win);
    await waitFor(() => expect(popup.location.href).toBe(started.authorize_url));
    expect(popup.opener).toBeNull();
    expect(calls[0]).toEqual(["settings.mcp.authorize", { scene: "code", project_root: null, name: "docs", origin: "http://127.0.0.1:4317" }]);
    // Still at the authorization server: nothing to hand over.
    tick();
    expect(calls.length).toBe(1);
    popup.location.href = "http://127.0.0.1:4317/ext/settings/mcp/callback?code=c&state=S";
    tick();
    await done;
    expect(calls[1]).toEqual(["settings.mcp.authComplete", { callback_url: popup.location.href }]);
    expect(popup.closed).toBe(true);
    expect(settled).toBe(1);
    expect(notices.at(-1)![0]).toContain("settings.mcpSignedIn");
  });

  it("says so when the window is kept from opening, or the engine will not start a sign-in", async () => {
    const blocked = host({});
    await signIn(blocked.h, "docs", { project_root: null }, () => undefined, { ...browser().win, open: () => null } as unknown as Window);
    expect(blocked.calls).toEqual([]);
    expect(blocked.notices[0]![0]).toContain("settings.mcpSignInBlocked");
    const { popup, win } = browser();
    const refused = host({ "settings.mcp.authorize": Object.assign(new Error("it does not use OAuth"), { code: -32602 }) });
    await signIn(refused.h, "files", { project_root: null }, () => undefined, win);
    expect(popup.closed).toBe(true);
    expect(refused.notices[0]).toEqual([expect.stringContaining("settings.mcpSignInFailed"), "error"]);
  });
});
