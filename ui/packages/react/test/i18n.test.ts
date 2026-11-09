/**
 * The two dictionaries (concept §5.9: keys, never literal copy): the same keys, the same
 * placeholders per key, nothing blank — so switching language can never show a key or lose a value.
 */

import { describe, expect, it } from "vitest";
import { DICTIONARIES, currentLocale, setLocale, t } from "../src/i18n/index";
import { en, uiStrings, zhCN } from "../src/strings";
import { hunks, lineDiff } from "../src/diff";

const placeholders = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();

describe("dictionaries", () => {
  const zh = DICTIONARIES["zh-CN"];
  const en = DICTIONARIES.en;

  it("have the same keys", () => {
    expect(Object.keys(zh).sort()).toEqual(Object.keys(en).sort());
  });

  it("have the same placeholders for every key", () => {
    const differ = Object.keys(en).filter((k) => JSON.stringify(placeholders(en[k]!)) !== JSON.stringify(placeholders(zh[k] ?? "")));
    expect(differ).toEqual([]);
  });

  it("have no blank values", () => {
    for (const [locale, dict] of Object.entries(DICTIONARIES)) {
      expect(Object.keys(dict).filter((k) => !dict[k]!.trim()), locale).toEqual([]);
    }
  });

  it("interpolate, fall back to the key, and switch", () => {
    setLocale("en");
    expect(currentLocale()).toBe("en");
    expect(t("composer.queued", { index: 2 })).toBe("Queued 2");
    expect(t("no.such.key")).toBe("no.such.key");
    setLocale("zh-CN");
    expect(t("common.cancel")).toBe(zh["common.cancel"]);
  });
});

describe("the conversation components' table", () => {
  it("is read from the dictionaries' chat.* keys, in each language, with no key left showing", () => {
    for (const table of [uiStrings("en"), uiStrings("zh-CN")]) {
      const said = Object.values(table).map((v) => (typeof v === "function" ? (v as (...a: unknown[]) => string)("x", "y") : v));
      expect(said.filter((v) => typeof v !== "string" || v.includes("chat."))).toEqual([]);
    }
    expect(en.permAllowOnce).toBe(DICTIONARIES.en["chat.permAllowOnce"]);
    expect(zhCN.permAllowOnce).toBe(DICTIONARIES["zh-CN"]["chat.permAllowOnce"]);
  });

  it("says a sentence that takes values whole, per case", () => {
    expect(en.messageRefused?.("too long", "hi")).toBe('"hi" was refused: too long');
    expect(en.messageRefused?.("", undefined)).toBe("Refused");
    expect(zhCN.messageRefused?.("太长", "你好")).toBe("「你好」被拒收：太长");
    expect(en.compacted({ before: 9, after: 3, saved: 1200 })).toBe("Context compacted: 9 → 3 messages, ~1200 tokens saved");
    expect(en.compacted({})).toBe("Compacted message");
    expect(en.permAnswered(en.permDecisionDeny, "Bash")).toBe("Denied: Bash");
    expect(zhCN.permAnswered(zhCN.permDecisionTimeout, "Bash")).toBe("超时未答，已按拒绝处理：Bash");
    expect(en.contextUsage(500, 1000)).toBe("Context 500 / 1,000 tokens (50%)");
  });
});

describe("a diff's folded lines", () => {
  it("are counted in the page's language", () => {
    const lines = lineDiff("a\nb\nc\nd\ne\nf\ng", "a\nb\nc\nd\ne\nf\nG");
    setLocale("en");
    expect(hunks(lines, 1)[0]).toEqual({ op: "gap", text: "… 5 unchanged lines" });
    setLocale("zh-CN");
    expect(hunks(lines, 1)[0]).toEqual({ op: "gap", text: "… 5 行未变" });
    setLocale("en");
  });
});
