/**
 * hello-app's interface: two spaces and a HOME card, registered through the same points Harnix's
 * own use. Compiled into the application's interface by the shell build (`virtual:harnix-modules`).
 */

import { useEffect, useState } from "react";
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
    </div>
  );
}

function GreetedCard() {
  const nest = useNest();
  const [count, setCount] = useState<number | null>(null);
  useEffect(() => nest.subscribe("app.greeter.greeted", undefined, (v) => setCount((v as { count?: number } | null)?.count ?? 0)), [nest]);
  return <div data-testid="greeted-count">{count === null ? "…" : `${count} greeted`}</div>;
}
