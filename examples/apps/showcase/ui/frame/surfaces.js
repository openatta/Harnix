// The showcase's interface in isolated frames: every surface it declares, drawn by this one page.
//
// `HarnixApp` (vendor/harnix-app-sdk.js, @harnix/app-sdk) is its only way out: the shell's
// bridge. It calls the backend's methods and follows its topics there, asks the shell for a
// modal, a confirmation, a notice, a link, a session — and is refused what its manifest does not
// list. `HarnixKit` (vendor/harnix-ui-kit.js, @harnix/ui-kit) gives it the shell's look, and
// follows the shell's theme and language.
//
// The board's buttons each show one thing the bridge does; the LIVE suite
// (`tests/live/ui/apps-frames.spec.ts`) presses them. "Crash" makes the page go away, as a crash
// does: the shell's frame catches it and offers a reload.
"use strict";

(async () => {
  const { h, button, input, card, empty, alert, render, follow } = HarnixKit;
  const app = await HarnixApp.connect();
  follow(app);
  const ID = app.surface.app;
  const method = (name) => `app.${ID}.${name}`;
  const root = document.getElementById("root");
  root.className = "";
  root.removeAttribute("role");

  const STRINGS = {
    en: {
      board: "The showcase's board",
      boardHint: "In an isolated frame: everything goes through the shell.",
      placeholder: "A note for the board",
      add: "Pin it",
      compose: "Compose…",
      empty: "Nothing on the board yet",
      bump: "Count one more",
      heartbeat: "Start the heartbeat",
      overreach: "Reach too far",
      clear: "Clear the board",
      clearAsk: "Take every note off the board?",
      notify: "Say hello",
      hello: "hello from the board",
      crash: "Crash",
      theme: "Theme",
      escapes: "Escape pressed here",
      scratch: "Something to keep while away",
      heartbeatTab: "Show the heartbeat tab",
      ticks: "Heartbeat ticks",
      visible: "Visible",
      notes: "Notes on the board",
      newSession: "New conversation here",
      sessions: "Conversations in this space",
      close: "Close",
      cancel: "Cancel",
      card: "Seven tools from the Showcase application, and a board in an isolated frame.",
      readme: "Read about it",
      settings: "What the backend holds",
    },
    "zh-CN": {
      board: "示例应用的留言板",
      boardHint: "在隔离的 iframe 里：一切都经过外壳。",
      placeholder: "给留言板写一条",
      add: "贴上",
      compose: "写一条…",
      empty: "留言板上还没有东西",
      bump: "再数一个",
      heartbeat: "开始心跳",
      overreach: "越权调用",
      clear: "清空留言板",
      clearAsk: "把留言板上的都撤下来？",
      notify: "打个招呼",
      hello: "留言板向你问好",
      crash: "崩溃",
      theme: "主题",
      escapes: "在这里按下 Esc 的次数",
      scratch: "离开时要留着的东西",
      heartbeatTab: "打开心跳页签",
      ticks: "心跳次数",
      visible: "可见",
      notes: "留言板上的条数",
      newSession: "在这里开个新对话",
      sessions: "这个 space 里的对话",
      close: "关闭",
      cancel: "取消",
      card: "示例应用的七个工具，和一块放在隔离 iframe 里的留言板。",
      readme: "了解更多",
      settings: "后端里有什么",
    },
  };
  const s = (key) => (STRINGS[app.env.locale] ?? STRINGS.en)[key] ?? STRINGS.en[key];
  const errorOf = (e) => `${e?.code ? `${e.code}: ` : ""}${e?.message ?? e}`;

  /** The board's notes: what the backend lists once, then what its topic pushes. */
  function notesOf(onChange) {
    let notes = [];
    app.call(method("notes.list")).then((r) => {
      notes = r.notes ?? [];
      onChange(notes);
    });
    app.subscribe(`app.${ID}.notes`, {}, (v) => {
      notes = v?.notes ?? [];
      onChange(notes);
    });
  }

  /* ── the space's centre: the board ── */
  function board() {
    let notes = [];
    let escapes = 0;
    const text = input({ placeholder: s("placeholder"), testid: "showcase-note-input" });
    text.classList.add("sc-wide");
    const scratch = input({ placeholder: s("scratch"), testid: "showcase-scratch" });
    // What changes is drawn into these; the rest is built once per language, so typing is never
    // interrupted by a value arriving.
    const listed = h("div");
    const counter = h("span", { testid: "showcase-counter" });
    const ticks = h("span", { testid: "showcase-ticks" });
    const confirmed = h("span", { testid: "showcase-confirmed" });
    const theme = h("span", { testid: "showcase-theme" });
    const accent = h("span", { className: "sc-swatch", testid: "showcase-accent" });
    const escaped = h("span", { testid: "showcase-escapes" }, "0");
    const refused = h("div");
    const failed = h("div");
    const fail = (e) => render(failed, alert(errorOf(e), { testid: "showcase-error" }));
    const run = (name, params, then) =>
      app.call(method(name), params).then((r) => {
        render(failed);
        then?.(r);
      }, fail);
    const add = (value) => run("notes.add", { text: value }, () => (text.value = ""));
    window.addEventListener("keydown", (e) => {
      if (e.key !== "Escape") return;
      escapes += 1;
      escaped.textContent = String(escapes);
    });
    const drawNotes = () =>
      render(
        listed,
        notes.length
          ? h(
              "ul",
              { className: "hk-list sc-notes", testid: "showcase-notes" },
              ...notes.map((n, i) =>
                h("li", { role: "button", testid: "showcase-note", onClick: () => app.secondary.open({ index: i, text: n.text }, String(n.text)) }, String(n.text)),
              ),
            )
          : empty(s("empty"), "showcase-notes-empty"),
      );
    const drawEnv = () => {
      theme.textContent = app.env.theme;
      accent.title = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
    };
    function build() {
      render(
        root,
        h(
          "div",
          { className: "hk-page hk-stack", testid: "showcase-board" },
          h("h1", { className: "hk-h1" }, s("board")),
          h("div", { className: "hk-muted" }, s("boardHint")),
          h(
            "form",
            { className: "hk-row", onSubmit: (e) => (e.preventDefault(), add(text.value)) },
            text,
            button(s("add"), { type: "submit", variant: "primary", testid: "showcase-note-add" }),
            button(s("compose"), {
              testid: "showcase-compose",
              onClick: async () => {
                const result = await app.openModal("compose", { placeholder: s("placeholder") });
                if (result?.text) add(result.text);
              },
            }),
          ),
          listed,
          h(
            "div",
            { className: "hk-row" },
            button(s("bump"), { testid: "showcase-bump", onClick: () => run("counter.bump", {}, (r) => (counter.textContent = String(r.counter))) }),
            counter,
            button(s("heartbeat"), { testid: "showcase-heartbeat-start", onClick: () => run("timer.start", { ms: 300, times: 3 }) }),
            ticks,
            button(s("heartbeatTab"), { testid: "showcase-open-tab", onClick: () => app.details.open("heartbeat") }),
          ),
          h(
            "div",
            { className: "hk-row" },
            button(s("overreach"), {
              testid: "showcase-overreach",
              onClick: () =>
                app.call("harnix.apps.list").then(
                  () => render(refused, alert("not refused", { testid: "showcase-refused" })),
                  (e) => render(refused, alert(errorOf(e), { testid: "showcase-refused" })),
                ),
            }),
            button(s("clear"), {
              variant: "danger",
              testid: "showcase-clear",
              onClick: async () => {
                const yes = await app.confirm(s("clearAsk"), { danger: true, confirm: s("clear") });
                confirmed.textContent = yes ? "yes" : "no";
                if (yes) run("notes.clear", {});
              },
            }),
            confirmed,
            button(s("notify"), { testid: "showcase-notify", onClick: () => app.notify(s("hello")) }),
            button(s("crash"), {
              variant: "danger",
              testid: "showcase-crash",
              // The page goes away, as a page that crashed does: the shell sees its frame lose it.
              onClick: () => location.replace("about:blank"),
            }),
          ),
          refused,
          h("div", { className: "hk-row hk-muted" }, `${s("theme")}: `, theme, accent, ` · ${s("escapes")}: `, escaped),
          scratch,
          failed,
        ),
      );
      drawNotes();
      drawEnv();
    }
    let locale = app.env.locale;
    app.onEnv((env) => {
      if (env.locale !== locale) {
        locale = env.locale;
        build();
      } else drawEnv();
    });
    notesOf((n) => {
      notes = n;
      void app.badge(notes.length || null);
      drawNotes();
    });
    app.subscribe(`app.${ID}.heartbeat`, {}, (v) => (ticks.textContent = String(v?.ticks ?? "")));
    build();
  }

  /* ── the space's left column ── */
  function sidebar() {
    let notes = [];
    let sessions = [];
    let made = "";
    const list = () =>
      app.call("harnix.sessions", { space: ID }).then(
        (r) => ((sessions = r.sessions ?? []), draw()),
        () => undefined,
      );
    notesOf((n) => ((notes = n), draw()));
    app.session.onChange(() => void list());
    app.onEnv(() => draw());
    void list();
    function draw() {
      render(
        root,
        h(
          "div",
          { className: "sc-side hk-stack", testid: "showcase-side" },
          h("div", { className: "hk-muted" }, `${s("notes")}: `, h("span", { testid: "showcase-side-notes" }, String(notes.length))),
          h("div", { className: "hk-muted" }, `${s("sessions")}: `, h("span", { testid: "showcase-side-sessions" }, String(sessions.length))),
          button(s("newSession"), {
            size: "sm",
            testid: "showcase-new-session",
            onClick: () =>
              app.session.create().then(
                (ref) => ((made = ref.sessionId), list()),
                (e) => ((made = errorOf(e)), draw()),
              ),
          }),
          h("span", { className: "hk-muted", testid: "showcase-made" }, made),
        ),
      );
    }
    draw();
  }

  /* ── the right column's tab ── */
  function heartbeatTab() {
    let ticks = "";
    app.subscribe(`app.${ID}.heartbeat`, {}, (v) => ((ticks = String(v?.ticks ?? "")), draw()));
    app.onEnv(() => draw());
    function draw() {
      render(
        root,
        h(
          "div",
          { className: "hk-page flush hk-stack sc-side", testid: "showcase-tab" },
          h("div", {}, `${s("ticks")}: `, h("span", { testid: "showcase-tab-ticks" }, ticks || "0")),
          h("div", { className: "hk-muted" }, `${s("visible")}: `, h("span", { testid: "showcase-tab-visible" }, String(app.env.visible))),
        ),
      );
    }
    draw();
  }

  /* ── a note, in the second panel or the details ── */
  function note() {
    app.onParams(() => draw());
    app.onEnv(() => draw());
    function draw() {
      const p = app.surface.params ?? {};
      render(
        root,
        h(
          "div",
          { className: "hk-page flush hk-stack sc-side", testid: "showcase-note-detail" },
          h("div", { className: "hk-h2", testid: "showcase-note-text" }, String(p.text ?? "")),
          app.surface.point === "space.secondary" ? button(s("close"), { size: "sm", testid: "showcase-note-close", onClick: () => app.secondary.close() }) : null,
        ),
      );
    }
    draw();
  }

  /* ── the home card ── */
  function homeCard() {
    app.onEnv(() => draw());
    function draw() {
      render(
        root,
        h(
          "div",
          { className: "hk-page flush", testid: "showcase-card" },
          h("div", { className: "hk-muted" }, s("card"), " ", h("a", { href: "https://github.com/xbitsmaster/Harnix/tree/main/examples/apps/showcase", testid: "showcase-readme" }, s("readme"))),
        ),
      );
    }
    draw();
  }

  /* ── a settings section ── */
  function settings() {
    let state = null;
    app.call(method("state")).then((r) => ((state = r), draw()));
    app.onEnv(() => draw());
    function draw() {
      render(
        root,
        card({
          title: s("settings"),
          testid: "showcase-settings",
          body: state ? h("pre", { className: "hk-muted", testid: "showcase-state" }, JSON.stringify({ notes: state.notes, counter: state.counter, ticks: state.ticks }, null, 2)) : HarnixKit.loading(),
        }),
      );
    }
    draw();
  }

  /* ── the modal: a note, answered to whoever opened it ── */
  function compose() {
    const text = input({ multiline: true, placeholder: app.surface.params?.placeholder ?? s("placeholder"), testid: "showcase-compose-text" });
    render(
      root,
      h(
        "form",
        { className: "hk-stack", testid: "showcase-compose-form", onSubmit: (e) => (e.preventDefault(), app.close({ text: text.value })) },
        text,
        h(
          "div",
          { className: "hk-row" },
          button(s("add"), { type: "submit", variant: "primary", testid: "showcase-compose-pin" }),
          button(s("cancel"), { testid: "showcase-compose-cancel", onClick: () => app.close(null) }),
        ),
      ),
    );
    text.focus();
  }

  const surfaces = {
    space: board,
    "space.sidebar": sidebar,
    "space.tab": heartbeatTab,
    "space.secondary": note,
    "details.panel": note,
    "home.card": homeCard,
    "settings.section": settings,
    modal: compose,
  };
  (surfaces[app.surface.point] ?? (() => render(root, empty(app.surface.point))))();
})();
