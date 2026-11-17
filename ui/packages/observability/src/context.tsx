/**
 * The right column's context tab, standing at the end of every session's strip: what the session's
 * next model call carries — the context's parts, the system blocks each with its source and size,
 * the tools, the model (`observability.context`), the skills and hooks in force
 * (`harnix.session.skills`, `.hooks`), each MCP server's tools and instructions
 * (`settings.mcp.context`), and what the project's trust holds back
 * (`harnix-app.sessions.projectStatus`), with the way to trust the project.
 *
 * Each part is read on its own and fails on its own: a method this product does not have (no
 * settings, no Harnix app) leaves its part out rather than the tab.
 */

import { useEffect, useState } from "react";
import { errorText, type McpContext, type SessionHooks, type SessionSkills } from "@harnix/core";
import { ContextReport, Disclosure, useT, useTabInfo, type ContextReportData, type HostSessionRef as SessionKey } from "@harnix/react";

import { Failure, sessionParams, type Observed } from "./shared.tsx";

/** `harnix-app.sessions.projectStatus`: the project's trust, and what it holds back (harnix-app.md §3.1). */
interface ProjectStatus {
  project_root?: string | null;
  trust: "trusted" | "untrusted" | "none" | string;
  categories?: string[];
  withheld?: { key: string; category: string; name: string | null }[];
}

/** What one part of the tab read: its value, its failure, or that the method is not there to call. */
type Part<T> = { state: "loading" } | { state: "ok"; value: T } | { state: "failed"; error: unknown } | { state: "absent" };

function absent(e: unknown): boolean {
  // The host refuses a method `harnix.reachable` does not list before it is sent (`contrib.methodNotDeclared`).
  const text = errorText(e);
  return /not declared|not reachable/i.test(text) || (e as { code?: number } | null)?.code === -32601;
}

function usePart<T>(read: (() => Promise<T>) | null, deps: unknown[], again: number): Part<T> {
  const [part, setPart] = useState<Part<T>>({ state: "loading" });
  useEffect(() => {
    if (!read) {
      setPart({ state: "absent" });
      return;
    }
    let alive = true;
    read()
      .then((value) => alive && setPart({ state: "ok", value }))
      .catch((e: unknown) => alive && setPart(absent(e) ? { state: "absent" } : { state: "failed", error: e ?? "error" }));
    return () => {
      alive = false;
    };
    // The reader is made anew at each draw; what it reads is named by `deps`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, again]);
  return part;
}

/** The tab's body: for the session whose surface it is in. */
export function ContextTab({ observed }: { observed: Observed }) {
  const { tab } = useTabInfo();
  if (!tab.session) return null;
  return <SessionContext observed={observed} session={tab.session} visible={tab.visible} />;
}

export function SessionContext({ observed, session, visible = true }: { observed: Observed; session: SessionKey; visible?: boolean }) {
  const tr = useT();
  const host = observed.host;
  const [again, setAgain] = useState(0);
  const params = sessionParams(observed, session);
  const key = [session.instance, session.sessionId];
  useEffect(() => {
    const offs = [
      host.events.onTurnSettled((e) => {
        if (e.session_id === session.sessionId) setAgain((n) => n + 1);
      }),
      host.subscribe("harnix-app.trust", undefined, () => setAgain((n) => n + 1)),
    ];
    return () => offs.forEach((off) => off());
  }, [host, session.sessionId]);
  const context = usePart<ContextReportData>(() => host.call("observability.context", params), key, again);
  const skills = usePart<SessionSkills>(() => host.call("harnix.session.skills", params), key, again);
  const hooks = usePart<SessionHooks>(() => host.call("harnix.session.hooks", params), key, again);
  const mcp = usePart<McpContext>(() => host.call("settings.mcp.context", params), key, again);
  const trust = usePart<ProjectStatus>(() => host.call("harnix-app.sessions.projectStatus", params), key, again);
  const [trusting, setTrusting] = useState(false);
  const trustProject = async (status: ProjectStatus) => {
    const names = (status.categories ?? []).map((c) => tr(`context.category.${c}`, { category: c })).join("、");
    const yes = await host.dialogs.confirm(tr("context.trustConfirm", { root: status.project_root ?? "", categories: names }), { title: tr("context.trust"), confirm: tr("context.trust") });
    if (!yes) return;
    setTrusting(true);
    try {
      await host.call("harnix-app.projects.trust", { project_root: status.project_root, trusted: true, instance: session.instance });
      setAgain((n) => n + 1);
    } catch (e) {
      host.shell.notice(errorText(e), "error");
    } finally {
      setTrusting(false);
    }
  };
  if (!visible && context.state === "loading") return null;
  return (
    <div className="obs-context" data-testid="context-tab">
      {context.state === "ok" ? (
        <>
          <div className="note">{tr("system.contextNote")}</div>
          <ContextReport context={context.value} />
        </>
      ) : context.state === "failed" ? (
        <Failure error={context.error} testId="context-report-failed" className="note" />
      ) : context.state === "absent" ? (
        <div className="note" data-testid="context-report-absent">
          {tr("context.noReport")}
        </div>
      ) : null}

      {trust.state === "ok" && trust.value.withheld?.length ? (
        <section className="obs-section" data-testid="context-withheld">
          <div className="label">{tr("context.withheld", { count: trust.value.withheld.length })}</div>
          <div className="note">{tr("context.withheldNote")}</div>
          <ul className="obs-list">
            {trust.value.withheld.map((item) => (
              <li key={item.key} data-category={item.category}>
                <span className="kd2">{tr(`context.category.${item.category}`, { category: item.category })}</span>
                <span className="txt">{item.name ?? item.key}</span>
              </li>
            ))}
          </ul>
          <button type="button" className="btn sm outline wide" data-testid="context-trust" disabled={trusting} onClick={() => void trustProject(trust.value as ProjectStatus)}>
            {tr("context.trust")}
          </button>
        </section>
      ) : trust.state === "failed" ? (
        <Failure error={trust.error} className="note" />
      ) : null}

      <section className="obs-section" data-testid="context-skills">
        <div className="label">{tr("context.skills", { count: skills.state === "ok" ? skills.value.skills.length : "—" })}</div>
        {skills.state === "ok" ? (
          skills.value.skills.length ? (
            <ul className="obs-list">
              {skills.value.skills.map((skill) => (
                <li key={`${skill.tier}/${skill.name}`} data-testid="context-skill" data-withheld={skill.withheld ? "true" : undefined} title={skill.description}>
                  <span className="kd2">{tr(`context.tier.${skill.tier}`)}</span>
                  <span className="txt">{skill.name}</span>
                  {skill.withheld ? <span className="kd warn">{tr("context.withheldTag")}</span> : null}
                </li>
              ))}
            </ul>
          ) : (
            <div className="note">{skills.value.listed ? tr("context.noSkills") : tr("context.skillsUnlisted")}</div>
          )
        ) : skills.state === "failed" ? (
          <Failure error={skills.error} className="note" />
        ) : skills.state === "absent" ? (
          <div className="note">{tr("context.unavailable")}</div>
        ) : null}
      </section>

      <section className="obs-section" data-testid="context-hooks">
        <div className="label">{tr("context.hooks", { count: hooks.state === "ok" ? hooks.value.hooks.length : "—" })}</div>
        {hooks.state === "ok" ? (
          hooks.value.hooks.length ? (
            <ul className="obs-list">
              {hooks.value.hooks.map((hook, i) => (
                <li key={i} data-testid="context-hook" data-in-force={hook.in_force ? "true" : "false"} className={hook.in_force ? "" : "muted"}>
                  <span className="kd2">{hook.event}</span>
                  <span className="txt">{hook.label}</span>
                  <span className="kd">{tr(`context.source.${hook.source}`)}</span>
                  {hook.withheld ? <span className="kd warn">{tr("context.withheldTag")}</span> : null}
                  {!hook.in_force && !hook.withheld ? <span className="kd">{hook.replaced_by ? tr("context.replacedBy", { source: tr(`context.source.${hook.replaced_by}`) }) : tr("context.notInForce")}</span> : null}
                </li>
              ))}
            </ul>
          ) : (
            <div className="note">{tr("context.noHooks")}</div>
          )
        ) : hooks.state === "failed" ? (
          <Failure error={hooks.error} className="note" />
        ) : hooks.state === "absent" ? (
          <div className="note">{tr("context.unavailable")}</div>
        ) : null}
        {hooks.state === "ok" && hooks.value.unread.length ? <div className="note">{tr("context.hooksUnread", { tiers: hooks.value.unread.map((s) => tr(`context.source.${s}`)).join("、") })}</div> : null}
      </section>

      <section className="obs-section" data-testid="context-mcp">
        <div className="label">{tr("context.mcp", { count: mcp.state === "ok" ? mcp.value.servers.length : "—" })}</div>
        {mcp.state === "ok" ? (
          mcp.value.servers.length ? (
            mcp.value.servers.map((server) => (
              <Disclosure
                key={server.name}
                testId="context-mcp-server"
                state={server.status?.state ?? "ok"}
                summary={
                  <>
                    <span className="hx-tool-name">{server.name}</span>
                    <span className="hx-tool-arg">{server.status?.state ?? ""}</span>
                  </>
                }
                tail={tr("context.mcpTools", { count: server.tools.length, tokens: server.tools.reduce((n, t) => n + t.tokens, 0) })}
              >
                <ul className="obs-list">
                  {server.tools.map((tool) => (
                    <li key={tool.name}>
                      <span className="txt mono">{tool.name}</span>
                      <span className="kd">{tr("system.tokens", { count: `~${tool.tokens}` })}</span>
                    </li>
                  ))}
                </ul>
                {server.instructions ? (
                  <>
                    <div className="label">{tr("context.mcpInstructions")}</div>
                    <pre className="hx-pre">{server.instructions}</pre>
                  </>
                ) : null}
              </Disclosure>
            ))
          ) : (
            <div className="note">{tr("context.noMcp")}</div>
          )
        ) : mcp.state === "failed" ? (
          <Failure error={mcp.error} className="note" />
        ) : mcp.state === "absent" ? (
          <div className="note">{tr("context.unavailable")}</div>
        ) : null}
      </section>
    </div>
  );
}
