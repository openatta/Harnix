var HarnixApp = (function(exports) {
  "use strict";
  const PROTOCOL = "harnix-ui-bridge@1";
  function shellKey(press) {
    const mod = press.meta || press.ctrl;
    if (press.key === "Escape" && !mod) return "escape";
    if (mod && !press.alt && press.key.toLowerCase() === "k") return "new";
    if (mod && !press.alt && press.key === "\\") return "sidebar";
    if (mod && !press.alt && press.key === "/") return "composer";
    return null;
  }
  function inDialog(point) {
    return point === "modal" || point === "settings.section";
  }
  function admits(app, hub, name, topic) {
    if (name.startsWith(`app.${app}.`)) return true;
    return hub.includes(topic ? `topic:${name}` : name);
  }
  const SDK_VERSION = "0.1.0";
  class BridgeError extends Error {
    code;
    data;
    constructor(error) {
      super(error.message);
      this.name = "BridgeError";
      this.code = error.code;
      this.data = error.data;
    }
  }
  let handed = null;
  const waiting = [];
  if (typeof window !== "undefined" && window.parent !== window) {
    const take = (event) => {
      const data = event.data;
      if (event.source !== window.parent || data?.protocol !== PROTOCOL || data.type !== "connect" || !event.ports[0]) return;
      window.removeEventListener("message", take);
      handed = event.ports[0];
      for (const fn of waiting.splice(0)) fn(handed);
    };
    window.addEventListener("message", take);
  }
  function shellPort(timeoutMs) {
    if (handed) return Promise.resolve(handed);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        const at = waiting.indexOf(done);
        if (at >= 0) waiting.splice(at, 1);
        reject(new BridgeError({ code: "timeout", message: "no Harnix shell handed this page a bridge: is it open outside Harnix?" }));
      }, timeoutMs);
      const done = (port) => {
        clearTimeout(timer);
        resolve(port);
      };
      waiting.push(done);
    });
  }
  async function connect(options = {}) {
    const timeoutMs = options.timeoutMs ?? 1e4;
    const port = options.port ?? await shellPort(timeoutMs);
    const app = new HarnixApp2(port);
    let timer;
    const welcome = await Promise.race([
      app.request("hello", { sdk: SDK_VERSION }),
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new BridgeError({ code: "timeout", message: "the shell did not answer hello" })), timeoutMs);
      })
    ]).finally(() => clearTimeout(timer));
    app.welcome(welcome, options);
    return app;
  }
  class HarnixApp2 {
    /** @internal Use `connect()`. */
    constructor(port) {
      this.port = port;
      port.addEventListener("message", this.receive);
      port.start?.();
    }
    port;
    next = 1;
    pending = /* @__PURE__ */ new Map();
    topics = /* @__PURE__ */ new Map();
    envListeners = /* @__PURE__ */ new Set();
    sessionListeners = /* @__PURE__ */ new Set();
    paramListeners = /* @__PURE__ */ new Set();
    info = null;
    environment = null;
    currentSession = null;
    cleanups = [];
    closed = false;
    /** What this frame is. */
    get surface() {
      return this.info;
    }
    /** The shell's look and state now. */
    get env() {
      return this.environment;
    }
    /** Follow the environment: theme, tokens, locale, narrow, visible, modal. Called on every change. */
    onEnv(fn) {
      this.envListeners.add(fn);
      return () => this.envListeners.delete(fn);
    }
    /** New params for this frame (a modal's, a secondary panel's, a details panel's item). */
    onParams(fn) {
      this.paramListeners.add(fn);
      return () => this.paramListeners.delete(fn);
    }
    /* ── data ── */
    /** A method: the application's own `app.<id>.*`, or one its `[interface] hub` lists. */
    call(method, params) {
      return this.request("call", { method, params });
    }
    /** A topic's values, as they are published; a refusal rejects `ready` and calls nothing. */
    subscribe(topic, params, fn) {
      let sub = null;
      let ended = false;
      const ready = this.request("subscribe", { topic, params }).then((r) => {
        sub = r.sub;
        if (ended) void this.request("unsubscribe", { sub }).catch(() => void 0);
        else this.topics.set(sub, fn);
      });
      const off = (() => {
        ended = true;
        if (sub === null) return;
        this.topics.delete(sub);
        void this.request("unsubscribe", { sub }).catch(() => void 0);
      });
      off.ready = ready;
      return off;
    }
    /** The application's space's sessions, as the shell has them open. */
    session = {
      current: () => this.currentSession,
      open: (ref) => this.request("session.open", ref).then(() => void 0),
      create: (spec = {}) => this.request("session.create", spec),
      onChange: (fn) => {
        this.sessionListeners.add(fn);
        return () => this.sessionListeners.delete(fn);
      }
    };
    /* ── the shell's hands ── */
    spaces = {
      switch: (space) => this.request("spaces.switch", { space }).then(() => void 0)
    };
    details = {
      open: (tab) => this.request("details.open", { tab }).then(() => void 0),
      show: (item) => this.request("details.show", { item }).then(() => void 0)
    };
    secondary = {
      open: (params, title) => this.request("secondary.open", { params, title }).then(() => void 0),
      close: () => this.request("secondary.close", {}).then(() => void 0)
    };
    settings = {
      open: (section) => this.request("settings.open", { section }).then(() => void 0)
    };
    composer = {
      insert: (text) => this.request("composer.insert", { text }).then(() => void 0)
    };
    /** A badge on the application's space entry; null, 0 or "" clears it. */
    badge(value) {
      return this.request("badge", { value }).then(() => void 0);
    }
    notify(text, level = "info") {
      return this.request("notify", { text, level }).then(() => void 0);
    }
    /** Text onto the clipboard, written by the shell: a frame has no clipboard of its own. */
    copy(text) {
      return this.request("copy", { text }).then(() => void 0);
    }
    /** One of the application's modals; resolves with what it closed with, null if it was dismissed. */
    openModal(id, params) {
      return this.request("openModal", { id, params });
    }
    /** In a modal: close it, answering its opener with `result`. */
    close(result) {
      this.event("close", { result });
    }
    /** The shell's own dialog, which no page can draw: true when the person confirmed. */
    confirm(text, options = {}) {
      return this.request("confirm", { text, ...options });
    }
    /** A web page, outside Harnix, after the person agrees (`[interface] links`); true when opened. */
    openLink(url) {
      return this.request("openLink", { url });
    }
    /** A file uploaded by the shell (a frame has no network): its path, for a message's attachments. */
    async upload(file) {
      const data = await file.arrayBuffer();
      return this.request("upload", { name: file.name ?? "upload", type: file.type, data }, [data]);
    }
    /** Stop talking to the shell (the page is going away). */
    disconnect() {
      if (this.closed) return;
      this.closed = true;
      for (const fn of this.cleanups.splice(0)) fn();
      this.port.removeEventListener("message", this.receive);
      this.port.close?.();
      for (const [, p] of this.pending) p.reject(new BridgeError({ code: "closed", message: "the bridge was closed" }));
      this.pending.clear();
    }
    /* ── the wire ── */
    /** @internal */
    request(method, params, transfer = []) {
      if (this.closed) return Promise.reject(new BridgeError({ code: "closed", message: "the bridge was closed" }));
      const id = this.next++;
      return new Promise((resolve, reject) => {
        this.pending.set(id, { resolve, reject });
        this.port.postMessage({ kind: "request", id, method, params }, transfer);
      });
    }
    event(name, data) {
      if (!this.closed) this.port.postMessage({ kind: "event", name, data });
    }
    receive = (event) => {
      const message = event.data;
      if (!message || typeof message !== "object") return;
      if (message.kind === "response") {
        const waiting2 = this.pending.get(message.id);
        if (!waiting2) return;
        this.pending.delete(message.id);
        if (message.error) waiting2.reject(new BridgeError(message.error));
        else waiting2.resolve(message.result);
      } else if (message.kind === "request") {
        const known = message.method === "ping";
        this.port.postMessage({
          kind: "response",
          id: message.id,
          ...known ? { result: "pong" } : { error: { code: "unknown", message: `no request \`${message.method}\` here` } }
        });
      } else if (message.kind === "event") {
        this.onEvent(message.name, message.data);
      }
    };
    onEvent(name, data) {
      if (name === "topic") {
        const { sub, value } = data;
        this.topics.get(sub)?.(value);
      } else if (name === "topic.end") {
        this.topics.delete(data.sub);
      } else if (name === "env") {
        const next = data;
        const before = this.environment;
        this.environment = next;
        const changed = Object.keys(next).filter((k) => JSON.stringify(before?.[k]) !== JSON.stringify(next[k]));
        if (changed.length) for (const fn of [...this.envListeners]) fn(next, changed);
      } else if (name === "session") {
        this.currentSession = data.current;
        for (const fn of [...this.sessionListeners]) fn(this.currentSession);
      } else if (name === "params") {
        const params = data.params;
        if (this.info) this.info = { ...this.info, params };
        for (const fn of [...this.paramListeners]) fn(params);
      }
    }
    /** @internal What `hello` answered: start following the shell. */
    welcome(welcome, options) {
      this.info = welcome.surface;
      this.environment = welcome.env;
      this.currentSession = welcome.current;
      if (typeof document === "undefined") return;
      if (options.keys !== false) this.cleanups.push(this.followKeys());
      if (options.autoHeight ?? welcome.surface.autoHeight) this.cleanups.push(this.reportHeight());
      this.cleanups.push(this.followLinks(), this.reportErrors());
    }
    /** Keys: the shell's go to it, and while one of its dialogs is over this frame, none go anywhere. */
    followKeys() {
      const onKey = (event) => {
        if (this.environment?.modal) {
          event.preventDefault();
          event.stopImmediatePropagation();
          return;
        }
        const press = { key: event.key, code: event.code, meta: event.metaKey, ctrl: event.ctrlKey, shift: event.shiftKey, alt: event.altKey };
        const which = shellKey(press);
        if (!which || which === "escape" && !inDialog(this.surface.point)) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        this.event("key", press);
      };
      window.addEventListener("keydown", onKey, true);
      return () => window.removeEventListener("keydown", onKey, true);
    }
    /** The content's height, whenever it changes, at most once a frame. */
    reportHeight() {
      let last = -1;
      let frame = 0;
      const measure = () => {
        frame = 0;
        const body = document.body;
        if (!body) return;
        const margin = parseFloat(getComputedStyle(body).marginBottom) || 0;
        const height = Math.ceil(body.getBoundingClientRect().bottom + margin + (window.scrollY || 0));
        if (height !== last) {
          last = height;
          this.event("resize", { height });
        }
      };
      const schedule = () => {
        if (!frame) frame = requestAnimationFrame(measure);
      };
      const observer = typeof ResizeObserver === "function" ? new ResizeObserver(schedule) : null;
      if (document.body) observer?.observe(document.body);
      window.addEventListener("load", schedule);
      schedule();
      return () => {
        observer?.disconnect();
        window.removeEventListener("load", schedule);
        if (frame) cancelAnimationFrame(frame);
      };
    }
    /**
     * A link to another site goes through the shell; this frame does not navigate itself (to the
     * shell, a frame whose page went away has failed). A link within the page — `#…` — is the
     * page's own.
     */
    followLinks() {
      const onClick = (event) => {
        const anchor = event.target?.closest?.("a[href]");
        if (!anchor || event.defaultPrevented) return;
        const here = new URL(location.href);
        const url = new URL(anchor.getAttribute("href") ?? "", here);
        if (url.hash && url.href.split("#")[0] === here.href.split("#")[0]) return;
        event.preventDefault();
        const web = url.protocol === "https:" || url.protocol === "http:";
        if (web && url.origin !== here.origin && this.surface.links) void this.openLink(url.href).catch(() => void 0);
      };
      document.addEventListener("click", onClick);
      return () => document.removeEventListener("click", onClick);
    }
    /** What the page did not catch, told to the shell — a few a second at most. */
    reportErrors() {
      let budget = 5;
      const refill = setInterval(() => budget = 5, 1e3);
      const report = (message) => {
        if (budget-- > 0) this.event("error", { message: message.slice(0, 500) });
      };
      const onError = (event) => report(event.message || String(event.error));
      const onRejection = (event) => report(String(event.reason?.message ?? event.reason));
      window.addEventListener("error", onError);
      window.addEventListener("unhandledrejection", onRejection);
      return () => {
        clearInterval(refill);
        window.removeEventListener("error", onError);
        window.removeEventListener("unhandledrejection", onRejection);
      };
    }
  }
  const DEFAULT_ENV = {
    theme: "light",
    tokens: {},
    locale: "en",
    narrow: false,
    visible: true,
    modal: false
  };
  function createMockHost(options = {}) {
    const app = options.app ?? "acme.notes";
    const hub = options.hub ?? [];
    const channel = new MessageChannel();
    const shell = channel.port1;
    const env = { ...DEFAULT_ENV, ...options.env };
    let current = options.current ?? null;
    const surface = {
      app,
      point: options.point ?? "space",
      id: options.id ?? options.point ?? "space",
      space: options.space === void 0 ? app : options.space,
      params: options.params ?? null,
      autoHeight: options.autoHeight ?? false,
      links: options.links ?? false
    };
    const subs = /* @__PURE__ */ new Map();
    let nextSub = 1;
    let nextPing = 1;
    const pings = /* @__PURE__ */ new Map();
    const requests = [];
    const events = [];
    const refused = [];
    const awaiting = [];
    const send = (message) => shell.postMessage(message);
    const refuse = (name, reason) => {
      refused.push({ name, reason });
      return { code: "refused", message: reason };
    };
    const answer = async (method, params) => {
      const p = params ?? {};
      switch (method) {
        case "hello":
          return { result: { surface, env, current } };
        case "call": {
          const name = String(p.method);
          if (!admits(app, hub, name, false)) return { error: refuse(name, `\`${name}\` is not ${app}'s to call`) };
          const handler = options.methods?.[name];
          if (!handler) return { error: { code: "failed", message: `no method \`${name}\` in this mock` } };
          try {
            return { result: await handler(p.params) };
          } catch (e) {
            return { error: { code: "failed", message: e instanceof Error ? e.message : String(e) } };
          }
        }
        case "subscribe": {
          const topic = String(p.topic);
          if (!admits(app, hub, topic, true)) return { error: refuse(topic, `\`${topic}\` is not ${app}'s to subscribe to`) };
          const sub = nextSub++;
          subs.set(sub, topic);
          return { result: { sub } };
        }
        case "unsubscribe":
          subs.delete(Number(p.sub));
          return { result: null };
        case "session.current":
          return { result: current };
        default: {
          const handler = options.requests?.[method];
          if (handler) {
            try {
              return { result: await handler(params) ?? null };
            } catch (e) {
              return { error: { code: "failed", message: e instanceof Error ? e.message : String(e) } };
            }
          }
          if (["notify", "badge", "copy", "spaces.switch", "details.open", "details.show", "secondary.open", "secondary.close", "settings.open", "composer.insert", "session.open"].includes(method)) {
            return { result: null };
          }
          if (method === "confirm" || method === "openLink") return { result: false };
          if (method === "openModal") return { result: null };
          return { error: { code: "unknown", message: `no request \`${method}\`` } };
        }
      }
    };
    shell.onmessage = (event) => {
      const message = event.data;
      if (message.kind === "request") {
        requests.push({ method: message.method, params: message.params });
        void answer(message.method, message.params).then((r) => send({ kind: "response", id: message.id, ...r }));
      } else if (message.kind === "event") {
        events.push({ name: message.name, data: message.data });
        for (const w of awaiting.filter((a) => a.name === message.name)) {
          awaiting.splice(awaiting.indexOf(w), 1);
          w.resolve(message.data);
        }
      } else if (message.kind === "response") {
        pings.get(message.id)?.(message.result ?? message.error);
        pings.delete(message.id);
      }
    };
    return {
      port: channel.port2,
      requests,
      events,
      refused,
      get env() {
        return env;
      },
      publish(topic, value) {
        for (const [sub, name] of subs) if (name === topic) send({ kind: "event", name: "topic", data: { sub, value } });
      },
      setEnv(change) {
        Object.assign(env, change);
        send({ kind: "event", name: "env", data: { ...env } });
      },
      setCurrent(next) {
        current = next;
        send({ kind: "event", name: "session", data: { current } });
      },
      setParams(params) {
        surface.params = params;
        send({ kind: "event", name: "params", data: { params } });
      },
      ping() {
        const id = nextPing++;
        return new Promise((resolve) => {
          pings.set(id, resolve);
          send({ kind: "request", id, method: "ping", params: {} });
        });
      },
      next(name) {
        const seen = events.find((e) => e.name === name);
        if (seen) return Promise.resolve(seen.data);
        return new Promise((resolve) => awaiting.push({ name, resolve }));
      },
      close() {
        shell.close();
      }
    };
  }
  exports.BridgeError = BridgeError;
  exports.HarnixApp = HarnixApp2;
  exports.PROTOCOL = PROTOCOL;
  exports.SDK_VERSION = SDK_VERSION;
  exports.admits = admits;
  exports.connect = connect;
  exports.createMockHost = createMockHost;
  exports.inDialog = inDialog;
  exports.shellKey = shellKey;
  Object.defineProperty(exports, Symbol.toStringTag, { value: "Module" });
  return exports;
})({});
