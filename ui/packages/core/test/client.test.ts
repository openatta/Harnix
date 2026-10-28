import { afterEach, describe, expect, it, vi } from "vitest";
import { CONTRIB_API_VERSION, HarnixClient, PROTOCOL_VERSION, TOKEN_STALE, tokenFromDocument, tokenFromPage, type ConnectionStatus } from "../src/index.ts";
import { FakeServer, FakeSplit, RpcFailure, settle } from "./fakes.ts";

const BASE = "http://127.0.0.1:4287";

function makeClient(server: FakeServer, split?: FakeSplit, extra: Partial<ConstructorParameters<typeof HarnixClient>[0]> = {}) {
  const statuses: ConnectionStatus[] = [];
  const client = new HarnixClient({
    token: "tok en",
    baseUrl: BASE,
    WebSocket: server.WebSocket,
    fetch: split?.fetch ?? (async () => new Response("no", { status: 404 })),
    EventSource: split?.EventSource,
    backoff: () => 10,
    ...extra,
  });
  client.onStatus((status) => statuses.push(status));
  return { client, statuses };
}

afterEach(() => {
  vi.useRealTimers();
});

describe("单双向流", () => {
  it("握手先于一切：带令牌连 /ws，谈定后状态为 open", async () => {
    const server = new FakeServer();
    const { client, statuses } = makeClient(server);
    client.connect();
    const negotiated = await client.ready();

    expect(server.last.url).toBe("ws://127.0.0.1:4287/ws?token=tok+en");
    expect(server.sent[0]).toMatchObject({
      method: "harnix.handshake",
      params: { protocol_version: PROTOCOL_VERSION, contrib_api_version: CONTRIB_API_VERSION, topology: "single_duplex" },
    });
    expect(negotiated.topology).toBe("single_duplex");
    expect(client.topology).toBe("single_duplex");
    expect(statuses).toEqual(["connecting", "open"]);
    client.close();
  });

  it("https 基址用 wss", async () => {
    const server = new FakeServer();
    const { client } = makeClient(server, undefined, { baseUrl: "https://nest.local:9000" });
    client.connect();
    await client.ready();
    expect(server.last.url.startsWith("wss://nest.local:9000/ws?token=")).toBe(true);
    client.close();
  });

  it("call 按 id 配对应答；错误以 {code, message} reject", async () => {
    const server = new FakeServer();
    server.answer = (method, params) => {
      if (method === "harnix.handshake") return { topology: "single_duplex", protocol_version: 4, contrib_api_version: CONTRIB_API_VERSION };
      if (method === "harnix.attach") return { session: { session_id: params.session_id }, seq: 7 };
      if (method === "harnix.send") return new RpcFailure({ code: -32602, message: "empty message" });
      return {};
    };
    const { client } = makeClient(server);
    client.connect();
    await client.ready();

    const attached = await client.call("harnix.attach", { session_id: "S1" });
    expect(attached.seq).toBe(7);
    await expect(client.call("harnix.send", { session_id: "S1", message: " " })).rejects.toEqual({
      code: -32602,
      message: "empty message",
    });
    const ids = server.sent.map((f) => f.id);
    expect(new Set(ids).size).toBe(ids.length);
    client.close();
  });

  it("未连接时 call 立即 reject，不排队", async () => {
    const server = new FakeServer();
    const { client } = makeClient(server);
    await expect(client.call("harnix.hello")).rejects.toMatchObject({ message: "not connected" });
  });

  it("通知分发给 onNotify 与带类型的 on；notify 不带 id", async () => {
    const server = new FakeServer();
    const { client } = makeClient(server);
    client.connect();
    await client.ready();

    const all: string[] = [];
    const seqs: number[] = [];
    client.onNotify((method) => all.push(method));
    const off = client.on("harnix.event", (params) => seqs.push(params.seq));
    server.last.push("harnix.event", { session_id: "S1", seq: 1, turn_id: "t", event: { kind: "text_delta", text: "a" } });
    server.last.push("harnix.queue", { session_id: "S1", items: [] });
    off();
    server.last.push("harnix.event", { session_id: "S1", seq: 2, turn_id: "t", event: { kind: "text_delta", text: "b" } });

    expect(all).toEqual(["harnix.event", "harnix.queue", "harnix.event"]);
    expect(seqs).toEqual([1]);

    client.notify("harnix.detach", { session_id: "S1" });
    const last = server.sent[server.sent.length - 1]!;
    expect(last).toMatchObject({ method: "harnix.detach", params: { session_id: "S1" } });
    expect(last.id).toBeUndefined();
    client.close();
  });

  it("answer 在单双向流上沿同一条连接回去", async () => {
    const server = new FakeServer();
    const { client } = makeClient(server);
    client.connect();
    await client.ready();
    await client.answer("session.respondToPrompt", { session_id: "S1", prompt_id: "p1", decision: { type: "permit" } });
    expect(server.sent.at(-1)).toMatchObject({ method: "session.respondToPrompt", params: { prompt_id: "p1" } });
    client.close();
  });

  it("版本不符被拒：incompatible，理由保留，不重连", async () => {
    const server = new FakeServer();
    server.answer = () => new RpcFailure({ code: -31001, message: "protocol version 4 is not 5; the server is out of date" });
    const { client, statuses } = makeClient(server);
    client.connect();
    await expect(client.ready()).rejects.toMatchObject({ message: expect.stringContaining("out of date") });
    await settle(5);

    expect(client.status).toBe("incompatible");
    expect(client.detail).toContain("out of date");
    expect(server.sockets).toHaveLength(1);
    expect(statuses).not.toContain("closed");
  });

  it("掉线：挂起的调用被 reject，按退避重连并重新握手", async () => {
    const server = new FakeServer();
    server.answer = (method) => (method === "harnix.handshake" ? { topology: "single_duplex" } : undefined);
    const { client, statuses } = makeClient(server);
    client.connect();
    await client.ready();

    const pending = client.call("harnix.hello");
    server.last.drop();
    await expect(pending).rejects.toMatchObject({ message: "connection closed" });
    expect(client.status).toBe("closed");

    await new Promise((resolve) => setTimeout(resolve, 40)); // 退避是 10ms
    await settle(3);
    expect(server.sockets).toHaveLength(2);
    expect(client.status).toBe("open");
    expect(server.sent.filter((f) => f.method === "harnix.handshake")).toHaveLength(2);
    expect(statuses).toEqual(["connecting", "open", "closed", "connecting", "open"]);
    client.close();
  });

  it("close 之后不再重连", async () => {
    vi.useFakeTimers();
    const server = new FakeServer();
    const { client } = makeClient(server);
    client.connect();
    await vi.runAllTimersAsync();
    expect(client.status).toBe("open");
    client.close();
    await vi.runAllTimersAsync();
    expect(server.sockets).toHaveLength(1);
    expect(client.status).toBe("closed");
  });
});

describe("分离流回退", () => {
  it("握手说拓扑不提供：换分离流，只连一次，调用走 /rpc 与 /respond，帧从 SSE 来", async () => {
    const server = new FakeServer();
    server.answer = () =>
      new RpcFailure({ code: -31001, message: "topology `single_duplex` is not served here; this build offers split_streams" });
    const split = new FakeSplit();
    split.rpc = (method) => (method === "harnix.sessions" ? { sessions: [{ session_id: "S1" }] } : { prompt_id: "p1" });
    const { client } = makeClient(server, split);
    client.connect();
    await client.ready();
    await settle(5);

    expect(client.topology).toBe("split_streams");
    // 上游 client.js 在这里会因 onclose 再走一次回退；移植版只握手一次。
    expect(split.requests.filter((r) => r.url.endsWith("/handshake"))).toHaveLength(1);
    expect(split.requests[0]!.body).toMatchObject({ token: "tok en", topology: "split_streams", protocol_version: 4 });
    expect(split.sources.map((s) => new URL(s.url).pathname)).toEqual(["/events/session", "/events/host"]);
    expect(split.sources[0]!.url).toContain("token=cred-1");

    const listed = await client.call("harnix.sessions");
    expect(listed.sessions[0]!.session_id).toBe("S1");
    await client.answer("session.respondToPrompt", { session_id: "S1", prompt_id: "p1", decision: { type: "permit" } });
    expect(split.requests.map((r) => new URL(r.url).pathname)).toEqual(["/handshake", "/rpc", "/respond"]);

    const seen: unknown[] = [];
    client.on("harnix.queue", (params) => seen.push(params.items));
    split.sources[0]!.push("harnix.queue", { session_id: "S1", items: [{ item_id: "q1", message: "hi" }] });
    expect(seen).toEqual([[{ item_id: "q1", message: "hi" }]]);

    client.notify("harnix.detach", { session_id: "S1" });
    expect(split.requests.at(-1)).toMatchObject({ keepalive: true, body: { method: "harnix.detach" } });

    split.sources[1]!.fail();
    expect(client.status).toBe("degraded");
    client.close();
    expect(split.sources.every((s) => s.es.closed)).toBe(true);
  });

  it("连接从未打开：去问另一个端点", async () => {
    const server = new FakeServer();
    server.autoOpen = false;
    const split = new FakeSplit();
    const { client } = makeClient(server, split);
    client.connect();
    server.last.drop();
    await client.ready();
    expect(client.topology).toBe("split_streams");
    client.close();
  });

  it("分离流握手也被拒：incompatible，理由取自服务端", async () => {
    const server = new FakeServer();
    server.autoOpen = false;
    const split = new FakeSplit();
    split.handshake = { error: { code: -31001, message: "contribution API version 1 is not 2" } };
    const { client } = makeClient(server, split);
    client.connect();
    server.last.drop();
    await expect(client.ready()).rejects.toMatchObject({ message: "contribution API version 1 is not 2" });
    expect(client.status).toBe("incompatible");
  });

  it("首选分离流时直接走它", async () => {
    const server = new FakeServer();
    const split = new FakeSplit();
    const { client } = makeClient(server, split, { speaks: ["split_streams"] });
    client.connect();
    await client.ready();
    expect(server.sockets).toHaveLength(0);
    expect(client.topology).toBe("split_streams");
    client.close();
  });
});

describe("令牌", () => {
  it("从页面的 meta 读出", () => {
    const doc = {
      querySelector: (selector: string) =>
        selector === 'meta[name="harnix-token"]' ? ({ getAttribute: () => "abc" } as unknown as Element) : null,
    };
    expect(tokenFromDocument(doc)).toBe("abc");
    expect(tokenFromDocument({ querySelector: () => null })).toBe("");
  });
});

describe("服务换了令牌", () => {
  /** 一台会"重启"的假服务端：只认 `token` 这个令牌，别的一律在升级时拒掉（浏览器只看到 onclose）。 */
  function restartable(server: FakeServer) {
    const state = { token: "tok en" };
    server.autoOpen = false;
    const Base = server.WebSocket;
    const WebSocket = class extends Base {
      constructor(url: string) {
        super(url);
        const socket = server.last;
        const given = new URL(url).searchParams.get("token");
        queueMicrotask(() => (given === state.token ? socket.open() : socket.drop()));
      }
    };
    return { state, WebSocket };
  }

  it("带 refreshToken：重连失败时取到新令牌，换上再连，回到 open", async () => {
    const server = new FakeServer();
    const { state, WebSocket } = restartable(server);
    const refresh = vi.fn(async () => state.token);
    const { client } = makeClient(server, undefined, { WebSocket, refreshToken: refresh });
    client.connect();
    await client.ready();
    state.token = "new token";
    server.last.drop();
    await expect.poll(() => client.status, { timeout: 2000 }).toBe("open");
    expect(refresh).toHaveBeenCalledTimes(1);
    expect(new URL(server.last.url).searchParams.get("token")).toBe("new token");
    client.close();
  });

  it("不带 refreshToken：一直用旧令牌，连不回去", async () => {
    const server = new FakeServer();
    const { state, WebSocket } = restartable(server);
    const { client } = makeClient(server, undefined, { WebSocket });
    client.connect();
    await client.ready();
    state.token = "new token";
    server.last.drop();
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(client.status).not.toBe("open");
    expect(server.sockets.length).toBeGreaterThan(2);
    expect(server.sockets.every((s) => new URL(s.url).searchParams.get("token") === "tok en")).toBe(true);
    client.close();
  });

  it("取不到新令牌、连续失败够多次：closed 带 TOKEN_STALE，重连照旧", async () => {
    const server = new FakeServer();
    const { state, WebSocket } = restartable(server);
    const details: (string | undefined)[] = [];
    const { client } = makeClient(server, undefined, { WebSocket, refreshToken: async () => null });
    client.onStatus((_s, detail) => details.push(detail));
    client.connect();
    await client.ready();
    state.token = "new token";
    server.last.drop();
    await expect.poll(() => details.includes(TOKEN_STALE), { timeout: 3000 }).toBe(true);
    client.close();
  });

  it("tokenFromPage 同源取页面、读 meta；不认（401）或连不上时是 null", async () => {
    const page = (status: number, body = "") => async () => new Response(body, { status });
    const html = '<html><head><meta name="harnix-token" content="abc 123"></head></html>';
    expect(await tokenFromPage(BASE, page(200, html))()).toBe("abc 123");
    expect(await tokenFromPage(BASE, page(401))()).toBeNull();
    expect(await tokenFromPage(BASE, async () => Promise.reject(new Error("refused")))()).toBeNull();
  });
});
