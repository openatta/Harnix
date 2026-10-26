// A contributed tool row, contribution API 2.
//
// Precompiled — no JSX, no bundler on the way in — and importing React by name: the page's import
// map sends `react/jsx-runtime` and `@harnix/react` to the shell's own copies, so the hooks below
// run on the same React as everything else and `useNest()` finds the shell's host.
//
// Registered after the built-ins, so it wins the claim for `Bash` and leaves every other tool to
// the row the product draws. Tokens only, keys only: its text comes from its own language pack,
// whose keys carry its name.
import { jsx, jsxs } from "react/jsx-runtime";
import { useNest } from "@harnix/react";

function Row({ block }) {
  const host = useNest();
  return jsxs("div", {
    className: "hx-blk",
    "data-testid": "demo-row",
    children: [jsx("span", { className: "name", children: host.t("demo-rows-v2.label") }), jsx("span", { className: "summary", children: String(block.name || "") })],
  });
}

export function register(registry) {
  registry.strings("en", { "demo-rows-v2.label": "demo-rows v2" });
  registry.strings("zh-CN", { "demo-rows-v2.label": "demo-rows 第二版" });
  registry.register("tool.row", {
    id: "demo-rows-v2.bash",
    match: (block) => block.name === "Bash",
    render: (block) => jsx(Row, { block }),
  });
}
