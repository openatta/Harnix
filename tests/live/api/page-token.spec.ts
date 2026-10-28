/**
 * The page's token after a restart (ipdrun-15, the backend's half).
 *
 * The token is one per process and travels only in the page's HTML. What a page that outlived its
 * backend can count on: the old token is refused, and the page fetched again — with the same
 * authority it was loaded with — carries the new one, uncached. That is all `tokenFromPage()` in
 * the client needs; nothing is served at a new endpoint.
 */

import { CONTRIB_API_VERSION, PROTOCOL_VERSION } from "@harnix/core/protocol";
import { expect, startHarnix, launchFor, test } from "@harnix/testkit";

const TAGS = { tag: ["@feature:connection", "@layer:api"] };

async function pageToken(url: string): Promise<{ token: string | null; cache: string | null }> {
  const response = await fetch(url);
  const html = await response.text();
  const token = /<meta name="harnix-token" content="([^"]*)"/.exec(html)?.[1] ?? null;
  return { token, cache: response.headers.get("cache-control") };
}

async function handshake(port: number, token: string): Promise<number> {
  const response = await fetch(`http://127.0.0.1:${port}/handshake`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      token, protocol_version: PROTOCOL_VERSION, contrib_api_version: CONTRIB_API_VERSION, topology: "split_streams",
    }),
  });
  return response.status;
}

test("after a restart the page carries the new token, and the old one is refused", TAGS, async () => {
  const first = await startHarnix(launchFor("replay"));
  let next = first;
  try {
    const before = await pageToken(first.url);
    expect(before.token).toBe(first.token);
    expect(before.cache).toBe("no-store");

    next = await first.restart();
    expect(next.port).toBe(first.port);
    expect(next.token).not.toBe(first.token);
    const after = await pageToken(next.url);
    expect(after.token).toBe(next.token);
    expect(await handshake(next.port, first.token)).toBe(401);
    expect(await handshake(next.port, next.token)).toBe(200);
  } finally {
    await next.dispose();
  }
});
