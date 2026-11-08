/**
 * Launch links (`[transport] launch_link = true`, `--launch-link`): the page carries no token; a
 * program holding the token mints a one-time link, and opening it sets a session cookie that
 * stands in for the token — at the page, and at connect time.
 *
 *   POST /auth/link   Authorization: Bearer <token>  →  {code, path, expires_in_secs}
 *   GET  /auth/{code} once                            →  200, Set-Cookie: harnix_session=…
 */

import { CONTRIB_API_VERSION, PROTOCOL_VERSION } from "@harnix/core/protocol";
import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

const TAGS = { tag: ["@feature:connection", "@layer:api"] };

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay", { args: ["--launch-link"], topologies: ["split_streams"] }));
});
test.afterAll(async () => { await backend?.dispose(); });

/** `path` on the backend (its `url` ends in `/`). */
const at = (path: string) => new URL(path, backend.url).toString();

const mint = (authorization?: string) =>
  fetch(at("/auth/link"), { method: "POST", headers: authorization ? { authorization } : {} });

/** The handshake a page makes, carrying `cookie` and no token of its own. */
const handshake = (cookie?: string) =>
  fetch(at("/handshake"), {
    method: "POST",
    headers: { "content-type": "application/json", ...(cookie ? { cookie } : {}) },
    body: JSON.stringify({ token: "", protocol_version: PROTOCOL_VERSION, contrib_api_version: CONTRIB_API_VERSION, topology: "split_streams" }),
  });

test("without a launch link the page is refused, and says how to get one", TAGS, async () => {
  const page = await fetch(at("/"));
  expect(page.status).toBe(401);
  expect(await page.text()).toMatch(/launch link/i);
  expect((await handshake()).ok).toBe(false);
});

test("only the token holder mints a link", TAGS, async () => {
  expect((await mint()).status).toBe(401);
  expect((await mint("Bearer not-the-token")).status).toBe(401);
  const minted = await mint(`Bearer ${backend.token}`);
  expect(minted.status).toBe(200);
  expect(minted.headers.get("cache-control")).toBe("no-store");
  const link = await minted.json();
  expect(link.code).toMatch(/^[0-9a-f]{64}$/);
  expect(link.path).toBe(`/auth/${link.code}`);
  expect(link.expires_in_secs).toBeGreaterThan(0);
});

test("a link opens once; its cookie admits the page and the handshake, with no token in the page", TAGS, async () => {
  const { path } = await (await mint(`Bearer ${backend.token}`)).json();
  const opened = await fetch(at(path), { redirect: "manual" });
  expect(opened.status).toBe(200);
  const setCookie = opened.headers.get("set-cookie") ?? "";
  expect(setCookie).toMatch(/^harnix_session=[0-9a-f]+;/);
  expect(setCookie).toMatch(/HttpOnly/);
  expect(setCookie).toMatch(/SameSite=Strict/);
  const cookie = setCookie.split(";")[0]!;

  const page = await fetch(at("/"), { headers: { cookie } });
  expect(page.status).toBe(200);
  const html = await page.text();
  expect(html).not.toContain(backend.token);
  expect(/<meta name="harnix-token" content="([^"]*)"/.exec(html)?.[1]).toBe("");

  const shaken = await handshake(cookie);
  expect(shaken.status, await shaken.clone().text()).toBe(200);

  // Spent: the same link a second time, and a guessed cookie, are refused alike.
  expect((await fetch(at(path))).status).toBe(401);
  expect((await fetch(at("/"), { headers: { cookie: "harnix_session=guess" } })).status).toBe(401);
  expect((await handshake("harnix_session=guess")).ok).toBe(false);
});
