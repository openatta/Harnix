// The showcase's interface module: contribution API 2, like an installed package's [[ui]].
//
// Served from `/apps/example.showcase/ui/showcase.js` and imported by the page; `react` and
// `@harnix/react` resolve through the page's import map to the shell's own copies. Everything it
// registers is signed with the application's id, so its strings are `example.showcase.…`.
//
// Two things: a `home.card`, and the showcase's space (`[[space]]` in its manifest), whose centre
// is the backend's board. The board talks to the backend only through the page's own connection:
// it calls `app.example.showcase.*` methods and subscribes to the backend's topics, admitted and
// audited like any other call this page makes.
import { useEffect, useState } from "react";
import { jsx, jsxs } from "react/jsx-runtime";
import { useNest } from "@harnix/react";

const ID = "example.showcase";
const method = (name) => `app.${ID}.${name}`;
const topic = (name) => `app.${ID}.${name}`;

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

/** A topic's latest value, kept current. */
function useTopic(host, name) {
  const [value, setValue] = useState(null);
  useEffect(() => host.subscribe(topic(name), {}, (v) => setValue(v ?? null)), [host, name]);
  return value;
}

function Board() {
  const host = useNest();
  const pushed = useTopic(host, "notes");
  const heartbeat = useTopic(host, "heartbeat");
  const [listed, setListed] = useState(null);
  const [text, setText] = useState("");
  const [counter, setCounter] = useState(null);
  const [error, setError] = useState(null);

  // What is on the board when the space opens; every change after that arrives on the topic.
  useEffect(() => {
    host.call(method("notes.list"), {}).then((r) => setListed(r.notes ?? []), (e) => setError(String(e?.message ?? e)));
  }, [host]);
  const notes = pushed?.notes ?? listed ?? [];
  const run = (name, params, then) =>
    host.call(method(name), params).then(
      (r) => {
        setError(null);
        then?.(r);
      },
      (e) => setError(String(e?.message ?? e)),
    );

  return jsxs("div", {
    className: "settings-page",
    "data-testid": "showcase-board",
    children: [
      jsx("h2", { children: host.t("example.showcase.board") }),
      jsxs("form", {
        onSubmit: (e) => {
          e.preventDefault();
          run("notes.add", { text }, () => setText(""));
        },
        children: [
          jsx("input", {
            "data-testid": "showcase-note-input",
            value: text,
            placeholder: host.t("example.showcase.placeholder"),
            onChange: (e) => setText(e.target.value),
          }),
          jsx("button", { type: "submit", "data-testid": "showcase-note-add", children: host.t("example.showcase.add") }),
        ],
      }),
      jsx("ul", {
        "data-testid": "showcase-notes",
        children: notes.map((n, i) => jsx("li", { "data-testid": "showcase-note", children: String(n.text) }, i)),
      }),
      jsxs("div", {
        children: [
          jsx("button", {
            type: "button",
            "data-testid": "showcase-bump",
            onClick: () => run("counter.bump", {}, (r) => setCounter(r.counter)),
            children: host.t("example.showcase.bump"),
          }),
          jsx("span", { "data-testid": "showcase-counter", children: counter == null ? "" : ` ${counter}` }),
        ],
      }),
      jsxs("div", {
        children: [
          jsx("button", {
            type: "button",
            "data-testid": "showcase-heartbeat-start",
            onClick: () => run("timer.start", { ms: 300, times: 3 }),
            children: host.t("example.showcase.heartbeat"),
          }),
          jsx("span", { "data-testid": "showcase-ticks", children: heartbeat ? ` ${heartbeat.ticks}` : "" }),
        ],
      }),
      error ? jsx("div", { className: "dd", role: "alert", "data-testid": "showcase-error", children: error }) : null,
    ],
  });
}

export function register(registry, host) {
  registry.strings("en", {
    "example.showcase.title": "Showcase",
    "example.showcase.body": "Seven tools from the Showcase application: try “what can the showcase do?”",
    "example.showcase.space": "Showcase",
    "example.showcase.board": "The showcase's board",
    "example.showcase.placeholder": "A note for the board",
    "example.showcase.add": "Pin it",
    "example.showcase.bump": "Count one more",
    "example.showcase.heartbeat": "Start the heartbeat",
  });
  registry.strings("zh-CN", {
    "example.showcase.title": "示例应用",
    "example.showcase.body": "示例应用的七个工具：问问“这个示例应用能做什么？”",
    "example.showcase.space": "示例应用",
    "example.showcase.board": "示例应用的留言板",
    "example.showcase.placeholder": "给留言板写一条",
    "example.showcase.add": "贴上",
    "example.showcase.bump": "再数一个",
    "example.showcase.heartbeat": "开始心跳",
  });
  registry.register("home.card", {
    id: "example.showcase.card",
    order: 50,
    render: () => jsx(Card, {}),
  });
  registry.register("space", {
    id: ID,
    icon: "file",
    order: 60,
    title: () => host.t("example.showcase.space"),
    center: "hidden",
    overview: () => jsx(Board, {}),
  });
}
