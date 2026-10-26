/**
 * A backend running `harnix-testapp`: the built-ins plus modules that
 * succeed and fail on purpose (crates/testapp). Shared by the module,
 * route and migration suites.
 */

import { type Backend, launchFor, startHarnix } from "@harnix/testkit";

export const PROBE_PROFILE = `
[modules.probe]
greeting = "hi there"
key = { credential = "probe/key" }

[modules.nobody]
anything = 1
`;

export function startTestApp(extra: { seed?: (scratch: string) => void } = {}): Promise<Backend> {
  return startHarnix(
    launchFor("replay", {
      bin: "harnix-testapp",
      profile: PROBE_PROFILE,
      env: { HARNIX_CREDENTIAL_PROBE_KEY: "s3cret-value" },
      ...extra,
    }),
  );
}

/** The diagnostics page's entry for one module. */
export async function moduleStatus(api: { call(m: string, p?: unknown): Promise<any> }, id: string) {
  const report = await api.call("harnix.diagnostics");
  return report.modules.find((m: any) => m.id === id && (id !== "probe" || m.version === "0.0.1"));
}
