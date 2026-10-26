/**
 * hello-app's interface: two spaces and a HOME card, registered through the same points Harnix's
 * own use. Compiled into the application's interface by the shell build (`virtual:harnix-modules`).
 *
 * The greeter's tab also shows the channels for what a call and a topic do not fit: a long job and
 * its progress topic, whose result lands in the conversation as a card (an entry, drawn by the
 * `flow.block` below), and the module's own link — an HTTP route and a WebSocket.
 */

import { useEffect, useRef, useState } from "react";
import { useNest, type NestHost, type Registry } from "@harnix/react";

export function register(r: Registry, _host: NestHost): void {
  r.register(
    "space",
    {
      id: "greeter",
      icon: "globe",
      order: 20,
      title: () => "Greeter",
      center: "chat",
      tabs: [{ id: "greet", title: () => "Greet", render: () => <GreetTab /> }],
    },
    "hello-app",
  );
  r.register("space", { id: "notes", icon: "file", order: 30, title: () => "Notes", center: "chat" }, "hello-app");
  r.register("home.card", { id: "greeter.count", order: 10, title: () => "Greeter", render: () => <GreetedCard /> }, "hello-app");
  r.register(
    "flow.block",
    {
      id: "greeter.job_done",
      match: (block: Shape) => entryOf(block, "job_done") !== null,
      render: (block: Shape) => <JobDoneCard entry={entryOf(block, "job_done")!} />,
    },
    "hello-app",
  );
}

/** The `custom` block an entry becomes, as far as this module needs it. */
type Shape = { kind: string; type?: string; data?: unknown };
interface GreeterEntry {
  id: string;
  ns: string;
  event: string;
  payload: Record<string, unknown>;
}

/** The greeter's entry of this kind, if that is what the block is. */
function entryOf(block: Shape, event: string): GreeterEntry | null {
  if (block.kind !== "custom" || block.type !== "entry") return null;
  const entry = block.data as GreeterEntry | null;
  return entry?.ns === "app.greeter" && entry.event === event ? entry : null;
}

function JobDoneCard({ entry }: { entry: GreeterEntry }) {
  return (
    <div className="hx-blk" data-testid="job-done-card" data-entry={entry.id}>
      ✓ {String(entry.payload.job_id)}: counted to {String(entry.payload.total)}
    </div>
  );
}

function GreetTab() {
  const nest = useNest();
  const [name, setName] = useState("world");
  const [answer, setAnswer] = useState<string | null>(null);
  return (
    <div data-testid="greet-tab">
      <input value={name} onChange={(e) => setName(e.target.value)} />
      <button
        type="button"
        className="btn sm outline"
        data-testid="greet"
        onClick={() =>
          nest
            .call<{ greeting: string; count: number }>("app.greeter.greet", { name })
            .then((r) => setAnswer(`${r.greeting} (${r.count})`))
            .catch((e: Error) => setAnswer(e.message))
        }
      >
        Greet
      </button>
      {answer ? <div data-testid="greeting">{answer}</div> : null}
      <button type="button" className="btn sm outline" data-testid="greet-insert" onClick={() => nest.composer.insert(`Say hello to ${name}.`)}>
        Ask the Agent
      </button>
      <Job />
      <OwnLink />
    </div>
  );
}

/** A long job: the call answers with an id at once, the progress arrives on `app.greeter.job`. */
function Job() {
  const nest = useNest();
  const [job, setJob] = useState<string | null>(null);
  const [progress, setProgress] = useState<{ done: number; total: number; finished: boolean } | null>(null);
  useEffect(() => (job ? nest.subscribe("app.greeter.job", { job_id: job }, (v) => setProgress(v as typeof progress)) : undefined), [nest, job]);
  const start = async () => {
    const session = nest.session.current();
    const started = await nest.call<{ job_id: string }>("app.greeter.count_to", {
      to: 20,
      ...(session ? { instance: session.instance, session_id: session.sessionId } : {}),
    });
    setProgress(null);
    setJob(started.job_id);
  };
  return (
    <div data-testid="job">
      <button type="button" className="btn sm outline" data-testid="job-start" onClick={() => void start()}>
        Count to 20
      </button>
      {progress ? (
        <div data-testid="job-progress" data-finished={progress.finished}>
          <progress max={progress.total} value={progress.done} /> {progress.done}/{progress.total}
        </div>
      ) : null}
    </div>
  );
}

/** The module's own link: a route and a WebSocket under `/apps/greeter/`, the credential the host's. */
function OwnLink() {
  const nest = useNest();
  const [said, setSaid] = useState<string | null>(null);
  const [echoed, setEchoed] = useState<string | null>(null);
  const socket = useRef<WebSocket | null>(null);
  useEffect(() => () => socket.current?.close(), []);
  const fetchHello = async () => {
    const response = await nest.space("greeter").fetch("hello");
    setSaid(`${response.status} ${await response.text()}`);
  };
  const echo = () => {
    const ws = nest.space("greeter").socket("echo");
    socket.current = ws;
    ws.onopen = () => ws.send("ping");
    ws.onmessage = (event) => {
      setEchoed(String(event.data));
      ws.close();
    };
  };
  return (
    <div data-testid="own-link">
      <button type="button" className="btn sm outline" data-testid="route-fetch" onClick={() => void fetchHello()}>
        GET /apps/greeter/hello
      </button>
      {said ? <div data-testid="route-said">{said}</div> : null}
      <button type="button" className="btn sm outline" data-testid="route-echo" onClick={echo}>
        WebSocket echo
      </button>
      {echoed ? <div data-testid="route-echoed">{echoed}</div> : null}
    </div>
  );
}

function GreetedCard() {
  const nest = useNest();
  const [count, setCount] = useState<number | null>(null);
  useEffect(() => nest.subscribe("app.greeter.greeted", undefined, (v) => setCount((v as { count?: number } | null)?.count ?? 0)), [nest]);
  return <div data-testid="greeted-count">{count === null ? "…" : `${count} greeted`}</div>;
}
