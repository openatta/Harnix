/**
 * The two dictionaries (design §10.1: keys, never literal copy): the same keys, the same
 * placeholders per key, nothing blank — so switching language can never show a key or lose a value.
 */

import { describe, expect, it } from "vitest";
import { DICTIONARIES, currentLocale, setLocale, t } from "../src/i18n/index";

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
