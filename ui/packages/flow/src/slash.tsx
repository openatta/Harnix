/**
 * Two ways into a flow from where a project is worked on, besides "新任务" (`task.tsx`):
 *
 * - `/flow <flow> [subject]` in a conversation of a project — a `command` the composer offers and
 *   hands what was typed to (`submit`): another task of the conversation's project, started there
 *   with its main session shown in the conversation's space (`shown_in`), and a card in the
 *   conversation linking to it (`from_session`: the module writes the entry `task_started` there).
 *   The conversation itself stays what it was. With a subject and nothing else to ask for, the
 *   task starts at once; otherwise the new-task dialog opens on that flow, the subject filled in.
 *   A conversation of no project has no task to start: it says so.
 * - "为这个项目建流程" in the project's menu (`harnix-app.project.items`' `projectMenu`): the
 *   flows' general conversation, working in that project (`focus {project}`: what it creates and
 *   saves goes to `<project>/.harnix/flows/`), with a first message about the project put in its
 *   composer. A project's own flows are read only once it is trusted, so an untrusted one is
 *   offered trust first.
 *
 * The shell knows neither: the command is one of `command`'s, the card a `flow.block`.
 */

import { Glyph, t, useT, type CommandCandidate, type CommandContext, type HostApi, type HostSessionRef as SessionRef, type MenuItem, type Registry, type SpaceContribution } from "@harnix/react";

import { errorText, useRunView } from "./hooks";
import { defBlockedKey, defsOf, methodName, phaseOf, type BlockShape } from "./model";
import { fieldsOf, newTaskDialog } from "./NewTask";
import { PhaseBadge, taskStep } from "./RunPanel";
import type { FlowSpace } from "./space";
import type { DefSummary, RunSummary, StartedFrom } from "./types";

/** The command's name: `/flow`. */
export const SLASH_FLOW = "flow";
/** The entry the module writes into the conversation a task was started from. */
export const TASK_STARTED = "task_started";

/* ── the card ─────────────────────────────────────────────────────────── */

/** What a `task_started` entry says. */
export interface TaskCard {
  /** The entry's id. */
  id: string;
  run: string;
  flow: string;
  flowTitle: string;
  title: string;
  subject: string;
  projectRoot: string | null;
}

/** The card linking to a task started from this conversation, if the block is one. */
export function taskCardOf(block: BlockShape, ns = "flow"): TaskCard | null {
  if (block.kind !== "custom" || block.type !== "entry") return null;
  const entry = block.data as { id?: string; ns?: string; event?: string; payload?: unknown } | null;
  if (!entry || entry.ns !== ns || entry.event !== TASK_STARTED) return null;
  const p = entry.payload as Record<string, unknown> | null;
  if (!p || typeof p.run !== "string" || !p.run) return null;
  const str = (v: unknown) => (typeof v === "string" ? v : "");
  return {
    id: String(entry.id ?? ""),
    run: p.run,
    flow: str(p.flow),
    flowTitle: str(p.flow_title),
    title: str(p.title),
    subject: str(p.subject),
    projectRoot: typeof p.project_root === "string" ? p.project_root : null,
  };
}

/** The card: which flow, about what, where it is now; opening it opens the task. */
export function TaskLinkCard({ card, module, open }: { card: TaskCard; module: string; open(): void | Promise<void> }) {
  const t = useT();
  const view = useRunView(module, card.run);
  const phase = view ? phaseOf({ phase: view.phase, status: view.record.status }) : null;
  const step = taskStep(view);
  return (
    <div className="hx-blk hx-flow-task-card" data-testid="flow-task-card" data-reveal={card.id || undefined} data-run={card.run} data-flow={card.flow} data-phase={phase?.state}>
      <Glyph name="fork" className="glyph" />
      <div className="hx-flow-task-card-body">
        <div className="hx-flow-task-card-head">{t("flowui.slash.card", { flow: card.flowTitle || card.flow })}</div>
        <div className="hx-flow-task-card-subject" title={card.subject}>
          {card.title || card.subject || card.run}
        </div>
        {phase || step ? (
          <div className="meta">
            {phase ? <PhaseBadge phase={phase} /> : null}
            {step ? <span className="hx-flow-task-step">{t("flowui.task.step", { k: step.k, n: step.n })}</span> : null}
          </div>
        ) : null}
      </div>
      <button type="button" className="btn sm outline" data-testid="flow-task-card-open" onClick={() => void open()}>
        {t("flowui.slash.open")}
      </button>
    </div>
  );
}

/* ── /flow ────────────────────────────────────────────────────────────── */

/** The conversation a command is typed in: the open session, its project, and the backend space it belongs to. */
export interface Conversation {
  session: SessionRef;
  root: string | null;
  space: string | null;
}

/**
 * The conversation `ctx` names: its project as the session list (else its own state) says, and
 * its space as the list says, else the backend space the front space lists sessions of. Null
 * with no session open.
 */
export function conversationOf(host: HostApi, r: Registry, ctx: CommandContext): Conversation | null {
  const session = ctx.session;
  if (!session) return null;
  const str = (v: unknown) => (typeof v === "string" && v ? v : null);
  const row = host.session.list().find((s) => s.instance === session.instance && s.session_id === session.sessionId);
  const open = host.session.current();
  const own = open && open.instance === session.instance && open.sessionId === session.sessionId ? host.session.context()?.session : null;
  const front = r.at("space").find((c: SpaceContribution) => c.id === ctx.space);
  return {
    session,
    root: str(row?.project_root) ?? str(own?.project_root),
    space: str(row?.space) ?? str(front?.sessions),
  };
}

/** `<flow> [subject]`: the first word, and the rest as it was typed (trimmed). */
export function flowArgs(args: string): { flow: string; subject: string } {
  const m = /^(\S+)(?:\s+([\s\S]*))?$/.exec(args.trim());
  return m ? { flow: m[1]!, subject: (m[2] ?? "").trim() } : { flow: "", subject: "" };
}

/** Whether a flow asks for something before it can start: a required input with no default. */
export function asksForInput(def: Pick<DefSummary, "inputs">): boolean {
  return fieldsOf(def.inputs).some((f) => f.required && f.kind !== "boolean" && (f.default === undefined || f.default === null));
}

/** The `/` menu's candidates: `/flow` itself, then `/flow <name>` for each flow the project may start. */
export function slashCandidates(defs: DefSummary[], submit: CommandCandidate["submit"]): CommandCandidate[] {
  const base: CommandCandidate = { replacement: `/${SLASH_FLOW}`, label: SLASH_FLOW, description: t("flowui.slash.about"), submit };
  return [
    base,
    ...defs
      .filter((d) => d.valid !== false)
      .map((d) => ({
        replacement: `/${SLASH_FLOW} ${d.name}`,
        label: `${SLASH_FLOW} ${d.name}`,
        description: d.title && d.title !== d.name ? d.title : t("flowui.origin." + (d.origin ?? "builtin")),
      })),
  ];
}

/**
 * `/flow` as a `command`: offered with the flows of the conversation's project, and taking what
 * follows it when sent. `runs` answers the runs as last known (the dialog's warning and number).
 */
export function registerSlashFlow(r: Registry, host: HostApi, module: string, runs: () => RunSummary[], owner: string): void {
  const defsFor = async (root: string) => defsOf(await host.call(methodName(module, "defs"), { project: root }));

  const submit = async (args: string, ctx: CommandContext): Promise<boolean> => {
    const conv = conversationOf(host, r, ctx);
    if (!conv?.root) {
      host.shell.notice(t(conv ? "flowui.slash.noProject" : "flowui.slash.noConversation"));
      return false;
    }
    const root = conv.root;
    const { flow, subject } = flowArgs(args);
    // The conversation gets the card; one whose space is not known gets none.
    const from: StartedFrom | undefined = conv.space ? { instance: conv.session.instance, session_id: conv.session.sessionId, space: conv.space } : undefined;
    const dialog = (picked?: string) => {
      void newTaskDialog(host, module, { root, space: conv.space ?? undefined, runs, flow: picked, subject, from, registry: r }, () => undefined);
      return true;
    };
    if (!flow) return dialog();
    const def = (await defsFor(root)).find((d) => d.name === flow);
    if (!def) {
      host.shell.notice(t("flowui.slash.unknown", { flow }), "error");
      return false;
    }
    if (def.valid === false) {
      host.shell.notice(t("flowui.slash.blocked", { flow, why: t(defBlockedKey(def) ?? "flowui.task.invalid") }), "error");
      return false;
    }
    if (!subject || asksForInput(def)) return dialog(flow);
    await host.call(methodName(module, "start"), {
      flow,
      subject,
      project_root: root,
      shown_in: conv.space ?? undefined,
      from_session: from,
    });
    return true;
  };

  r.register(
    "command",
    {
      id: `${owner}.slash`,
      async complete(_query: string, ctx: CommandContext): Promise<CommandCandidate[]> {
        const root = conversationOf(host, r, ctx)?.root;
        if (!root) return slashCandidates([], submit);
        try {
          return slashCandidates(await defsFor(root), submit);
        } catch {
          return slashCandidates([], submit);
        }
      },
    },
    owner,
  );
}

/** The card of a task started from a conversation, as a `flow.block`; it opens the task as the projects space does. */
export function registerTaskCard(r: Registry, ns: string, module: string, open: (card: TaskCard) => void | Promise<void>, owner: string): void {
  r.register(
    "flow.block",
    {
      id: `${owner}.task-card`,
      match: (block: BlockShape) => taskCardOf(block, ns) !== null,
      render: (block: BlockShape) => {
        const card = taskCardOf(block, ns)!;
        return <TaskLinkCard card={card} module={module} open={() => open(card)} />;
      },
    },
    owner,
  );
}

/* ── 为这个项目建流程 ───────────────────────────────────────────────────── */

/** What `projectMenu` is handed about the project (`harnix-app.project.items`' context). */
export interface ProjectMenuContext {
  root: string;
  trust?: { status(): Promise<string | null>; ask(): Promise<boolean> };
}

/**
 * Whether flows may be saved in the project now: trusted, or trusted after asking. One whose
 * trust cannot be read is let through — the module refuses a save it should not make, and says why.
 */
export async function trustedForAuthoring(host: HostApi, context: ProjectMenuContext): Promise<boolean> {
  const trust = context.trust;
  if (!trust) return true;
  const status = await trust.status();
  if (status === "trusted" || status === null) return true;
  if (status === "none") {
    host.shell.notice(t("flowui.forProject.noLayer"), "error");
    return false;
  }
  const go = await host.dialogs.confirm(t("flowui.forProject.trustFirst", { path: context.root }), { title: t("flowui.forProject.title"), confirm: t("flowui.forProject.trust") });
  return go && (await trust.ask());
}

/**
 * The flows' general conversation, working in the project, brought up where the flows are (their
 * space, or the console's), with a first message about the project in its composer.
 */
export async function authorForProject(host: HostApi, r: Registry, space: FlowSpace, context: ProjectMenuContext): Promise<void> {
  if (!(await trustedForAuthoring(host, context))) return;
  const here = host.spaces.current();
  const from = r.at("space").find((c: SpaceContribution) => c.id === here);
  let title = here;
  try {
    title = from?.title() ?? here;
  } catch {
    // A space whose title fails is named by its id.
  }
  await space.state.enter(null, null, { from: from ? { space: from.id, title } : undefined });
  const ref = await space.state.openGeneral();
  if (!ref) return;
  await host.call(methodName(space.module, "focus"), { instance: ref.instance, session_id: ref.sessionId, flow: null, project: context.root });
  host.composer.insert(t("flowui.forProject.prompt", { path: context.root }), { session: ref });
}

/** The project menu's entry. */
export function authorMenuItem(host: HostApi, r: Registry, space: () => FlowSpace | null, context: ProjectMenuContext): MenuItem {
  return {
    id: "flow.author-for-project",
    icon: "fork",
    label: t("flowui.forProject.menu"),
    run: async () => {
      const view = space();
      if (!view) return;
      try {
        await authorForProject(host, r, view, context);
      } catch (e) {
        host.shell.notice(errorText(e), "error");
      }
    },
  };
}
