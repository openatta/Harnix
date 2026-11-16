/**
 * A model service that fails the way real ones do, for the suites that check how a failure is
 * told to a person: an Anthropic-shaped endpoint on a port of its own, answering every request with
 * the status the test set, or dropping the connection.
 *
 * It is a provider on the settings page (`settings.setProvider`, `api_type: "anthropic"`, its
 * `base_url`), so the engine's own client, its classing of the failure (`data.kind` of the turn's
 * error) and its retries are all the real ones; only the far end is made up. Run it on a backend
 * that is not replaying (`launchFor("none")`): replay answers instead of calling.
 */

import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";

import type { TestClient } from "@harnix/testkit";

export type Fault =
  | { status: number; type: string; message: string; headers?: Record<string, string> }
  | { drop: true };

export interface StubProvider {
  /** The provider's `base_url`. */
  url: string;
  /** What the next requests get, until it is set again. */
  fail(fault: Fault): void;
  /** How many requests came in. */
  requests(): number;
  close(): Promise<void>;
}

/** What the real services say for the faults the interface has words for. */
export const FAULTS = {
  auth: { status: 401, type: "authentication_error", message: "invalid x-api-key" },
  forbidden: { status: 403, type: "permission_error", message: "this key may not use this model" },
  billing: { status: 400, type: "invalid_request_error", message: "Your credit balance is too low to access the API" },
  wrongModel: { status: 404, type: "not_found_error", message: "model: no-such-model" },
  rateLimited: { status: 429, type: "rate_limit_error", message: "rate limit exceeded", headers: { "retry-after": "7" } },
  overloaded: { status: 529, type: "overloaded_error", message: "Overloaded" },
  drop: { drop: true },
} as const satisfies Record<string, Fault>;

export async function stubProvider(initial: Fault = FAULTS.auth): Promise<StubProvider> {
  let fault: Fault = initial;
  let count = 0;
  const server: Server = createServer((request, response) => {
    request.resume();
    request.on("end", () => {
      count += 1;
      if ("drop" in fault) {
        request.socket.destroy();
        return;
      }
      response.writeHead(fault.status, { "content-type": "application/json", ...fault.headers });
      response.end(JSON.stringify({ type: "error", error: { type: fault.type, message: fault.message } }));
    });
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;
  return {
    url: `http://127.0.0.1:${port}`,
    fail: (next) => {
      fault = next;
    },
    requests: () => count,
    close: () => new Promise<void>((resolve) => { server.closeAllConnections(); server.close(() => resolve()); }),
  };
}

/** Make the stub the engine's default provider, with `model` as its default model. */
export async function useStub(api: TestClient, stub: StubProvider, model = "stub-model"): Promise<void> {
  await api.call("settings.setProvider", {
    provider_id: "stub",
    config: { api_type: "anthropic", api_key: "sk-stub", base_url: stub.url, default_model: model, models: [model] },
    default_provider: "stub",
  });
}

/** Take the stub away again, so the next test starts from the profile's own providers. */
export async function dropStub(api: TestClient): Promise<void> {
  await api.call("settings.setProvider", { provider_id: "stub", delete: true });
}
