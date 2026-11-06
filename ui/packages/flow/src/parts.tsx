/**
 * The parts of a flow pack besides flows (xops-04): a station's rules and permissions, a flow's
 * 配套 — the skills, rules, policies and gates it uses, each openable to read — and the hooks over
 * it with each one's last delivery. Shown, never edited here: the conversation's tools write them.
 */

import { useT } from "@harnix/react";

import { useMethod } from "./hooks";
import { formatTime, partDir, partTitle, permissionScope, PART_KINDS } from "./model";
import type { DefInfo, Granted, HookInfo, PartKind } from "./types";

/** A station's permissions, in the order they are judged in; deny over ask over allow. */
export function PermissionList({ permissions, testid }: { permissions: Granted[]; testid?: string }) {
  const t = useT();
  if (!permissions.length) {
    return (
      <span className="hx-flow-faint" data-testid={testid}>
        {t("flowui.def.noPermissions")}
      </span>
    );
  }
  return (
    <ul className="hx-flow-list hx-flow-perms" data-testid={testid}>
      {permissions.map((p, i) => {
        const scope = permissionScope(p);
        return (
          <li key={i} data-effect={p.effect} data-testid={testid ? `${testid}-${i}` : undefined}>
            <span className={`hx-flow-pill hx-flow-effect hx-flow-tone-${p.effect === "allow" ? "ok" : p.effect === "ask" ? "warn" : "error"}`}>{t("flowui.effect." + p.effect)}</span>{" "}
            {p.tools.map((tool) => (
              <code key={tool} className="hx-flow-chip">
                {tool}
              </code>
            ))}
            {scope ? <span className="hx-flow-faint"> · {scope}</span> : null}
            <span className="hx-flow-faint"> · {p.from}</span>
            {p.message ? <div className="hx-flow-perm-msg">{p.message}</div> : null}
          </li>
        );
      })}
    </ul>
  );
}

/** Rule names as chips. */
export function RuleChips({ rules, info }: { rules: string[]; info?: DefInfo | null }) {
  const t = useT();
  if (!rules.length) return <span className="hx-flow-faint">{t("flowui.def.none")}</span>;
  return (
    <>
      {rules.map((r) => (
        <code key={r} className="hx-flow-chip" title={partTitle("rule", r, info?.parts?.rules?.[r])}>
          {r}
        </code>
      ))}
    </>
  );
}

/** The kinds a flow's 配套 lists, in order. */
const USED: PartKind[] = PART_KINDS.filter((k) => k !== "hook");

/** A flow's parts: each kind, each part openable to its text; then the hooks over the flow. */
export function FlowParts({ module, info, flow }: { module: string; info: DefInfo | null; flow: string }) {
  const t = useT();
  const parts = info?.parts ?? {};
  const any = USED.some((k) => Object.keys((parts as Record<string, Record<string, unknown>>)[partDir(k)] ?? {}).length);
  return (
    <div data-testid="flow-parts">
      {!any ? <p className="hx-flow-hint">{t("flowui.parts.none")}</p> : null}
      {USED.map((kind) => {
        const entries = Object.entries((parts as Record<string, Record<string, string | null>>)[partDir(kind)] ?? {});
        if (!entries.length) return null;
        return (
          <div key={kind} className="hx-flow-part-group" data-testid={`flow-parts-${kind}`}>
            <div className="hx-flow-label">{t("flowui.part.kinds." + kind)}</div>
            {entries.map(([name, text]) => (
              <details key={name} className="hx-flow-details" data-testid={`flow-part-${kind}-${name}`} data-missing={text === null ? "true" : undefined}>
                <summary>
                  <code className="hx-flow-chip">{name}</code> {partTitle(kind, name, text) !== name ? partTitle(kind, name, text) : ""}
                  {text === null ? <span className="hx-flow-badge hx-flow-tone-error">{t("flowui.parts.missing")}</span> : null}
                </summary>
                {text !== null ? <pre className={`hx-flow-code ${kind === "skill" || kind === "rule" ? "hx-flow-prose" : "hx-flow-mono"}`}>{text}</pre> : null}
              </details>
            ))}
          </div>
        );
      })}
      <FlowHooks module={module} info={info} flow={flow} />
    </div>
  );
}

/** The hooks of the pack over a flow, each with what it is told and its last delivery. */
export function FlowHooks({ module, info, flow }: { module: string; info: DefInfo | null; flow: string }) {
  const t = useT();
  // The last deliveries change as runs go: read again whenever the flow's details are.
  const all = useMethod<HookInfo[]>(module, "hooks", {}, info?.digest);
  const names = new Set((info?.hooks ?? []).map((h) => h.name));
  const hooks = (all.value ?? []).filter((h) => names.has(h.name) || (!info && (!h.flows?.length || h.flows.includes(flow))));
  const shown = hooks.length ? hooks : (info?.hooks ?? []).map((h) => ({ ...h }) as HookInfo);
  return (
    <div className="hx-flow-part-group" data-testid="flow-parts-hook">
      <div className="hx-flow-label">{t("flowui.parts.hooks")}</div>
      {!shown.length ? <p className="hx-flow-hint">{t("flowui.hooks.none")}</p> : null}
      <ul className="hx-flow-list">
        {shown.map((h) => (
          <li key={h.name} data-testid={`flow-hook-${h.name}`} data-last={h.last?.status ?? undefined}>
            <code className="hx-flow-chip">{h.name}</code> {h.title || ""}
            <div className="hx-flow-faint">
              {[h.on?.length ? t("flowui.hooks.on", { list: h.on.join(", ") }) : t("flowui.hooks.all"), `${h.kind} → ${h.action}`, h.signed ? t("flowui.hooks.signed") : ""].filter(Boolean).join(" · ")}
            </div>
            <div className={h.last?.status === "failed" ? "hx-flow-tone-error" : "hx-flow-faint"} data-testid={`flow-hook-last-${h.name}`}>
              {!h.last
                ? t("flowui.hooks.never")
                : h.last.status === "delivered"
                  ? t("flowui.hooks.delivered", { at: formatTime(h.last.at), event: h.last.event })
                  : t("flowui.hooks.failed", { at: formatTime(h.last.at), event: h.last.event, error: h.last.error ?? "" })}
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}
