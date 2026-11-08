var HarnixKit = (function(exports) {
  "use strict";
  function applyEnv(env, root = document.documentElement) {
    root.dataset.theme = env.theme;
    root.lang = env.locale;
    if (env.narrow) root.dataset.narrow = "";
    else delete root.dataset.narrow;
    for (const [name, value] of Object.entries(env.tokens)) {
      if (name.startsWith("--") && typeof value === "string") root.style.setProperty(name, value);
    }
  }
  function follow(app, root) {
    applyEnv(app.env, root);
    return app.onEnv((env) => applyEnv(env, root));
  }
  function h(tag, props = {}, ...children) {
    const el = document.createElement(tag);
    for (const [name, value] of Object.entries(props)) {
      if (value === void 0 || value === null || value === false) continue;
      if (name === "className") el.className = String(value);
      else if (name === "testid") el.dataset.testid = String(value);
      else if (/^on[A-Z]/.test(name) && typeof value === "function") el.addEventListener(name.slice(2).toLowerCase(), value);
      else if (name === "value" && "value" in el) el.value = String(value);
      else el.setAttribute(name, value === true ? "" : String(value));
    }
    el.append(...children.filter((c) => c !== null && c !== void 0 && c !== false).map((c) => typeof c === "number" ? String(c) : c));
    return el;
  }
  const classes = (...names) => names.filter(Boolean).join(" ");
  function button(label, options = {}) {
    return h(
      "button",
      {
        type: options.type ?? "button",
        className: classes("hk-btn", options.variant, options.size),
        onClick: options.onClick,
        disabled: options.disabled,
        testid: options.testid,
        title: options.title
      },
      label
    );
  }
  function input(options = {}) {
    const el = h(options.multiline ? "textarea" : "input", {
      className: "hk-input",
      type: options.multiline ? void 0 : options.type ?? "text",
      name: options.name,
      placeholder: options.placeholder,
      value: options.value,
      testid: options.testid
    });
    if (options.onInput) el.addEventListener("input", (event) => options.onInput(el.value, event));
    return el;
  }
  function field(label, control) {
    return h("label", { className: "hk-field" }, h("span", { className: "hk-label" }, label), control);
  }
  function card(options = {}) {
    const body = Array.isArray(options.body) ? options.body : [options.body];
    return h("section", { className: "hk-card", testid: options.testid }, options.title ? h("div", { className: "hk-card-title" }, options.title) : null, ...body);
  }
  function table(columns, rows, options = {}) {
    if (!rows.length && options.empty !== void 0) return empty(options.empty);
    const head = h("thead", {}, h("tr", {}, ...columns.map((c) => h("th", {}, c.label))));
    const body = h(
      "tbody",
      {},
      ...rows.map(
        (row) => h(
          "tr",
          { onClick: options.onRow ? () => options.onRow(row) : void 0 },
          ...columns.map((c) => h("td", {}, typeof c.cell === "function" ? c.cell(row) : String(row[c.cell] ?? "")))
        )
      )
    );
    return h("table", { className: "hk-table", testid: options.testid }, head, body);
  }
  function empty(text, testid) {
    return h("div", { className: "hk-empty", testid }, text);
  }
  function loading(text, testid) {
    return h("div", { className: "hk-loading", role: "status", testid }, text ?? "");
  }
  function alert(text, options = {}) {
    return h("div", { className: classes("hk-alert", options.tone === "info" && "info"), role: options.tone === "info" ? "status" : "alert", testid: options.testid }, text);
  }
  function render(parent, ...children) {
    parent.replaceChildren(...children.filter((c) => c !== null && c !== void 0 && c !== false).map((c) => typeof c === "number" ? String(c) : c));
  }
  exports.alert = alert;
  exports.applyEnv = applyEnv;
  exports.button = button;
  exports.card = card;
  exports.empty = empty;
  exports.field = field;
  exports.follow = follow;
  exports.h = h;
  exports.input = input;
  exports.loading = loading;
  exports.render = render;
  exports.table = table;
  Object.defineProperty(exports, Symbol.toStringTag, { value: "Module" });
  return exports;
})({});
