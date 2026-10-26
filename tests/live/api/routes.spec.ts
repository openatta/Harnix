/**
 * Module routes, `/apps/<id>/` (design §7.7): same origin, admitted by the
 * same token or credential, decided in the authorization table, refused
 * before the module's code runs.
 */

import { type Backend, connect, expect, test } from "@harnix/testkit";

import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:routes", "@layer:api"] };

let backend: Backend;

test.beforeAll(async () => {
  backend = await startTestApp();
});
test.afterAll(() => backend?.stop());

const url = (path: string) => `${backend.url.replace(/\/$/, "")}${path}`;

test("a route answers a request that carries the token", TAGS, async () => {
  const response = await fetch(url("/apps/probe/hello"), {
    headers: { authorization: `Bearer ${backend.token}` },
  });
  expect(response.status).toBe(200);
  expect(await response.text()).toBe("hello from probe");
});

test("and one that carries a handshake's credential", TAGS, async () => {
  const api = await connect(backend);
  try {
    const credential = api.handshake.credential;
    expect(credential).toBeTruthy();
    const response = await fetch(url(`/apps/probe/hello?credential=${credential}`));
    expect(response.status).toBe(200);
  } finally {
    api.close();
  }
});

test("a request with nothing is refused before the module runs", TAGS, async () => {
  expect((await fetch(url("/apps/probe/hello"))).status).toBe(401);
  const wrong = await fetch(url("/apps/probe/hello"), { headers: { authorization: "Bearer guess" } });
  expect(wrong.status).toBe(401);
});

test("a page from elsewhere is refused by its Origin", TAGS, async () => {
  const response = await fetch(url("/apps/probe/hello"), {
    headers: { authorization: `Bearer ${backend.token}`, origin: "https://evil.example" },
  });
  expect(response.status).toBe(403);
});

test("a disabled module's routes are not served", TAGS, async () => {
  const response = await fetch(url("/apps/fails-start/hello"), {
    headers: { authorization: `Bearer ${backend.token}` },
  });
  expect(response.status).not.toBe(200);
  expect(await response.text()).not.toContain("should not be served");
});
