/**
 * The kit: it follows the shell's environment through a connected app (the SDK's mock host), and
 * its components are plain DOM with the kit's classes.
 */

import { afterEach, describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { connect } from "@harnix/app-sdk";
import { createMockHost } from "@harnix/app-sdk/mock";

import { applyEnv, button, card, empty, follow, h, input, loading, table } from "../src/index.ts";

const tick = () => new Promise((r) => setTimeout(r, 0));

afterEach(() => {
  document.documentElement.removeAttribute("style");
  delete document.documentElement.dataset.theme;
});

describe("following the shell", () => {
  it("sets the theme, the language and the token values, and changes them as the shell does", async () => {
    const host = createMockHost({ env: { theme: "light", locale: "en", tokens: { "--accent": "#2563eb" } } });
    const app = await connect({ port: host.port, keys: false });
    try {
      const stop = follow(app);
      const root = document.documentElement;
      expect(root.dataset.theme).toBe("light");
      expect(root.lang).toBe("en");
      expect(root.style.getPropertyValue("--accent")).toBe("#2563eb");
      host.setEnv({ theme: "dark", locale: "zh-CN", tokens: { "--accent": "#4f8cff" } });
      await tick();
      expect(root.dataset.theme).toBe("dark");
      expect(root.lang).toBe("zh-CN");
      expect(root.style.getPropertyValue("--accent")).toBe("#4f8cff");
      stop();
      host.setEnv({ theme: "light" });
      await tick();
      expect(root.dataset.theme).toBe("dark");
    } finally {
      app.disconnect();
      host.close();
    }
  });

  it("sets only custom properties", () => {
    applyEnv({ theme: "light", locale: "en", narrow: true, visible: true, modal: false, tokens: { color: "red", "--x": "1px" } });
    const root = document.documentElement;
    expect(root.style.getPropertyValue("color")).toBe("");
    expect(root.style.getPropertyValue("--x")).toBe("1px");
    expect(root.dataset.narrow).toBe("");
  });
});

describe("components", () => {
  it("are elements with the kit's classes", () => {
    let clicked = 0;
    const b = button("Add", { variant: "primary", size: "sm", testid: "add", onClick: () => (clicked += 1) });
    b.click();
    expect(b.className).toBe("hk-btn primary sm");
    expect(b.dataset.testid).toBe("add");
    expect(clicked).toBe(1);
    const i = input({ placeholder: "A note", value: "x" }) as HTMLInputElement;
    expect(i.className).toBe("hk-input");
    expect(i.value).toBe("x");
    expect(card({ title: "Notes", body: empty("Nothing yet") }).textContent).toBe("NotesNothing yet");
    expect(loading("Loading").getAttribute("role")).toBe("status");
    const t = table([{ label: "Text", cell: "text" as const }], [{ text: "one" }, { text: "two" }]);
    expect([...t.querySelectorAll("td")].map((td) => td.textContent)).toEqual(["one", "two"]);
    expect(table([{ label: "Text", cell: "text" as const }], [], { empty: "None" }).className).toBe("hk-empty");
    expect(h("div", { hidden: true, title: null }).outerHTML).toBe('<div hidden=""></div>');
  });

  it("draw colours from the tokens only", () => {
    const css = readFileSync(join(process.cwd(), "src", "kit.css"), "utf8");
    expect(css).toContain('@import "@harnix/react/tokens.css"');
    expect(css.match(/#[0-9a-fA-F]{3,8}\b|rgba?\(/g)).toBeNull();
  });
});
