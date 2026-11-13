/**
 * 不受信的内容：扩展进程里的引擎的会话（attach 的应答标 `trust: "extension"`）。HTML 只是文字；链接不成为 `<a>`，
 * 网址照原样看得见；图片只画 `data:` 的。产品自己的会话照旧。
 */

import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Block } from "@harnix/core/state";
import { HarnixUiProvider, SessionView, TranscriptList, markdown } from "../src";
import { FakeClient, attached, flush } from "./fake-client";

const HOSTILE = [
  "<script>alert(1)</script>",
  '<img src="x" onerror="alert(2)">',
  '<a href="javascript:alert(3)">点我</a>',
  "[坏](javascript:alert(4)) 与 [好](https://e.x/a)",
].join("\n\n");

const PIXEL = "data:image/png;base64,iVBORw0KGgo=";

describe("markdown(untrusted)", () => {
  it("HTML 只是文字；链接没有 <a>，网址看得见；javascript: 也一样", () => {
    const html = markdown(HOSTILE, { untrusted: true });
    expect(html).not.toMatch(/<script|<img|<a[\s>]/);
    expect(html).toContain("&lt;script&gt;");
    expect(html).toContain('<span class="hx-inert-link">好</span> (<code>https://e.x/a</code>)');
    // 网址里的括号截断了它（受信的链接也一样），截到哪都只是文字
    expect(html).toContain('<span class="hx-inert-link">坏</span> (<code>javascript:alert(4</code>)');
  });

  it("![图](网址) 不成为图片也不成为链接", () => {
    const html = markdown("![图](https://e.x/p.png)", { untrusted: true });
    expect(html).not.toMatch(/<img|<a[\s>]/);
    expect(html).toContain("https://e.x/p.png");
  });

  it("受信的内容照旧：https 链接仍是新窗口打开的 <a>", () => {
    expect(markdown("[好](https://e.x/a)")).toContain('<a href="https://e.x/a" target="_blank" rel="noreferrer noopener">好</a>');
    expect(markdown("[好](https://e.x/a)", { untrusted: false })).toContain('<a href="https://e.x/a"');
  });
});

describe("会话视图里的不受信会话", () => {
  let server: FakeClient;

  beforeEach(() => {
    server = new FakeClient();
    server.handlers["engine.session.history"] = () => ({
      messages: [
        {
          role: "assistant",
          content: [
            { type: "text", text: HOSTILE },
            { type: "image", source: { type: "url", url: "https://e.x/track.png" } },
            { type: "image", source: { type: "base64", media_type: "image/png", data: "iVBORw0KGgo=" } },
          ],
        },
      ],
    });
  });

  afterEach(() => cleanup());

  async function mount(trust?: "extension") {
    server.handlers["harnix.session.attach"] = () => attached("S1", { history_total: 1, ...(trust ? { trust } : {}) });
    render(
      <HarnixUiProvider client={server.asClient()}>
        <SessionView sessionId="S1" />
      </HarnixUiProvider>,
    );
    await act(async () => {
      await flush();
    });
    return screen.getByTestId("conv");
  }

  it("trust: extension ⇒ 没有活的元素；外部图片不取，data: 图片照画", async () => {
    const conv = await mount("extension");
    const message = within(conv).getByTestId("conv-message");
    expect(message.querySelector("script, img, a, [onerror]")).toBeNull();
    expect(message.textContent).toContain("<script>alert(1)</script>");
    expect(message.textContent).toContain("https://e.x/a");
    expect(conv.querySelector("a[href]")).toBeNull();
    const images = within(conv).getAllByTestId("conv-image");
    expect(images.map((i) => i.dataset.state ?? "shown")).toEqual(["blocked", "shown"]);
    expect(images[0]!.querySelector("img")).toBeNull();
    expect(images[0]!.textContent).toContain("https://e.x/track.png");
    expect(images[1]!.querySelector("img")!.getAttribute("src")).toBe(PIXEL);
  });

  it("产品自己的会话照旧：https 链接与外部图片都画", async () => {
    const conv = await mount();
    const links = [...conv.querySelectorAll("a[href]")].map((a) => a.getAttribute("href"));
    expect(links).toEqual(["https://e.x/a"]);
    expect(conv.querySelector("script, [onerror]")).toBeNull();
    expect([...conv.querySelectorAll("img")].map((i) => i.getAttribute("src"))).toEqual(["https://e.x/track.png", PIXEL]);
  });
});

describe("TranscriptList 的 trust", () => {
  afterEach(() => cleanup());

  const BLOCKS: Block[] = [
    { key: 1, kind: "assistant", text: "见 [文档](https://e.x/doc)", streaming: false },
    { key: 2, kind: "think", text: "想想 [这里](https://e.x/t) <b>x</b>", startedAt: 0, endedAt: 1000 },
    { key: 3, kind: "image", src: "https://e.x/p.png" },
  ];

  it("extension：正文与思考里都没有 <a>，外部图片不取", () => {
    render(
      <HarnixUiProvider client={new FakeClient().asClient()}>
        <TranscriptList blocks={BLOCKS} trust="extension" />
      </HarnixUiProvider>,
    );
    const flow = screen.getByTestId("conv-flow");
    fireEvent.click(within(flow).getByTestId("conv-think"));
    expect(within(flow).getByTestId("conv-think-text").textContent).toContain("https://e.x/t");
    expect(flow.querySelector("a, img, b")).toBeNull();
    expect(within(flow).getByTestId("conv-image").dataset.state).toBe("blocked");
  });
});
