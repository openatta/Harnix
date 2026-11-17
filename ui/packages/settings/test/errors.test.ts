/** A refused write said in words a person can act on. */

import { afterEach, describe, expect, it } from "vitest";
import { ENGINE_PROJECT_NOT_FOUND, ENGINE_SETTINGS_LOCKED } from "@harnix/core";
import { setLocale } from "@harnix/react";

import { settingsError, writeFailed } from "../src/errors.ts";

afterEach(() => setLocale("en"));

describe("a refused settings write", () => {
  it("says plainly when the tier is held by another program or the project is gone", () => {
    setLocale("en");
    const locked = settingsError({ code: ENGINE_SETTINGS_LOCKED, message: "the settings directory /x is locked by another process; nothing was written" });
    expect(locked).toContain("nothing was written");
    expect(locked).not.toContain("/x");
    expect(settingsError({ code: ENGINE_PROJECT_NOT_FOUND, message: "project_root /p is not a directory" })).toContain("project's folder");
    setLocale("zh-CN");
    expect(writeFailed({ code: ENGINE_SETTINGS_LOCKED, message: "locked" })).toContain("占着");
    expect(settingsError({ code: ENGINE_PROJECT_NOT_FOUND, message: "x" })).toContain("项目的目录");
  });

  it("gives any other refusal as the backend said it", () => {
    setLocale("en");
    expect(settingsError({ code: -32602, message: "`permission_mode` does not accept that value" })).toBe("`permission_mode` does not accept that value");
  });
});
