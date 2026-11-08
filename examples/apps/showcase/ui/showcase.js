// The showcase's interface module: contribution API 2, like an installed package's [[ui]].
//
// Served from `/apps/example.showcase/ui/showcase.js` and imported by the page; `react` and
// `@harnix/react` resolve through the page's import map to the shell's own copies. Everything it
// registers is signed with the application's id, so its strings are `example.showcase.…`.
import { jsx, jsxs } from "react/jsx-runtime";
import { useNest } from "@harnix/react";

function Card() {
  const host = useNest();
  return jsxs("div", {
    "data-testid": "showcase-card",
    children: [
      jsx("div", { className: "dt", children: host.t("example.showcase.title") }),
      jsx("div", { className: "dd", children: host.t("example.showcase.body") }),
    ],
  });
}

export function register(registry) {
  registry.strings("en", {
    "example.showcase.title": "Showcase",
    "example.showcase.body": "Six tools from the Showcase application: try “what can the showcase do?”",
  });
  registry.strings("zh-CN", {
    "example.showcase.title": "示例应用",
    "example.showcase.body": "示例应用的六个工具：问问“这个示例应用能做什么？”",
  });
  registry.register("home.card", {
    id: "example.showcase.card",
    order: 50,
    render: () => jsx(Card, {}),
  });
}
