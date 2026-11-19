# Flow guide

A flow is a directed acyclic graph of **stations**: a **run** carries one **subject** (a code base, a requirement, a draft…) from the first station to the last. Each station has its **work** and its **exit conditions**; when they all hold, the run moves on. A flow definition is a YAML file; it says what is done and how it is judged — how to do the work well is in the **skills**.

The whole picture comes in five groups, read from what is done to how it is written:

1. **The skeleton** — what is done: flow, station, work and work items, exit conditions, output contract.
2. **Running** — who does it, how it goes: run, executor, main session, sub-agent, session of its own, lease, rework, pause.
3. **Constraints and checks** — what can stop work: rules, policies and permissions, gates, a person's decision.
4. **Interfaces to the outside** — how it meets other systems: hooks (progress going out), external work items (work claimed and handed in from outside), the application's own capabilities.
5. **Writing and trying** — how one is made: the pack and its parts, flow conversations, validation, dry runs.

## 1. The skeleton

A flow is a directed acyclic graph of stations; each station has its **work** and its **exit conditions**, and moves on when they all hold.

| Object | What it is |
|---|---|
| **Flow** | A definition: `format` (optional: the definition format it is written in, `1`; a library that reads an older one refuses it whole), `name`, `title`, `inputs` (what a run is started with), `main` (where the main session goes), `rules` / `policies` / `permissions` (what every station works under), `guard` (how calls are judged, see section 3), `output_dir` (where deliverables go, see section 2), `workspace` (whether a task works in the project itself or a worktree of it, see section 2), `stations`. Changing it does not affect runs already started: a run pins its definition, with the policies and gates it uses. |
| **Station** | The unit of judgement. `needs` orders stations (absent = the previous one in the list, `[]` = none); `when` skips it unless it holds; `hold: true` waits for someone to release it. |
| **Work** | What a station does, one of: `executor` (one work item), `tasks` (several, each with an id), `foreach` + `executor` (one work item for each entry of a list, `max_parallel` at once), or none (a checkpoint, judged on entry). |
| **Work item** | One piece of work. Session, sub-agent and program work items are dispatched by the flow; external ones wait to be claimed; a subflow work item starts a child run. Not a project's "task" (the project task of section 2). |
| **Exit condition** | All must hold: `output` (every work item's output satisfies the `output` contract), `gate` (a gate of the pack passes), `confirm` (someone confirms), `children` (every child run finished). |
| **Output contract** | A JSON Schema. What a session or sub-agent hands in must satisfy it; later stations refer to it as `${stations.<id>.output.<field>}`. |

## 2. Running

A **run** carries one subject through the graph; each work item is done by an **executor**, and the main session is the flow's main Agent.

| Object | What it is |
|---|---|
| **Run** | One execution of a flow on one subject. Its states are below. |
| **Project task** | A run in a project: started with a `project_root` (an existing absolute directory). It may also be given a `workspace` (where it works: `project`, the project itself, the default with a project; `worktree`, a worktree of it; `scratch`, a directory of the run's own, the default without one) and an `output_dir` (this start's output directory, over the flow's); the run records them, with how it was started (`trigger`: by hand, or on a schedule). `flow.list {project}` lists a project's tasks. A task's main session, its sessions of their own and its gates work in the project (in its worktree, for one run in a worktree); all Harnix writes there is the output directory and `.harnix/` — anything else is the Agent's doing. |
| **Output directory** (`output_dir`) | Where the deliverables go: the user's, never ignored. The definition writes a path template relative to the project, which may say `${flow}`, `${date}` (the local date it starts on, `YYYYMMDD`), `${seq}` (two digits: one more than the highest already there for the same project, flow and day), `${title}`, `${inputs.<k>}`; the default is `flows/${flow}-${date}${seq}`. A start's own wins over the flow's, the flow's over the default. Resolved and made at start, it must stay inside the project (no `..`, no absolute path outside it, no link leading out — the start is refused otherwise); a value's `/` and the like become `-`, so it never adds a level or climbs out. The run records the absolute path, the station's message tells the Agent its deliverables go there, and skills, permissions and gates refer to it as `${output_dir}`. A run of no project has it under its own directory, laid out the same. |
| **Process directory** | `<project>/.harnix/runs/<run>/`, what the software keeps of the run's progress: `task.json` (the flow, inputs, subject, trigger, status, each station in brief, the output directory — rewritten after every change), `flow.yaml` (the definition and parts pinned at start), `stations/<station>/` (`output.json` for one work item, `<task>.json` each for several; a command gate's `context.json` and `gate-<gate>.log`). The first task in a project writes `<project>/.harnix/.gitignore` with the one line `runs/` (nothing when the repository's rules ignore it already; one already there only gets it added, one that is a link is left alone; the engine adds its own lines to the same file, and the project's own flows in `.harnix/flows/` are left to be committed). `.harnix/` is the project's data directory, which Harnix shares with its engine; an application that embeds the flow library names its own. It is a projection: the ledger in the application's data directory is what counts, and a process directory that cannot be written leaves the run as it was. |
| **Worktree** | A flow that says `workspace: worktree` (the default is `project`), or a start that does (a start's word wins over the flow's): the task does not change the project directly but works in a worktree of its own outside the project (under the flow extension's data directory, in the application's state: `worktrees/<run>`), on the branch `harnix/flow/<run>` made from the project's `HEAD` at start. The project must be a git repository (not bare, with a commit) or the start is refused, saying why; `flow.workspace_info {project_root}` tells beforehand (`{git, reason?, branch?}`). A project in a subdirectory of its repository works in the same subdirectory of the worktree. The main session, sessions of their own, gates (`FLOW_WORKSPACE`) and the output directory are in the worktree, and `${workspace}` is it; inside or outside "the project" means inside or outside the worktree. The process directory does not move: it stays in the project's own `.harnix/runs/<run>/`, every run in one place. A child run works in its parent's worktree. Once the run is over (finished or cancelled) a person decides: **merge** (`flow.worktree.merge {run}`: what the task left uncommitted is committed on its branch, which then goes into the project's current branch — a fast-forward when it can be, else a merge commit made in a scratch worktree beside it and the project's branch fast-forwarded to that; a conflict, or uncommitted changes of the project's in the way, stop it with `CONFLICT` and the files, the project's tree untouched; merged, the worktree and the branch are removed), **keep the branch** (`flow.worktree.keep {run}`, the default: uncommitted work committed, the worktree directory removed, the branch left — it may still be merged or discarded later), **discard** (`flow.worktree.discard {run}`: the worktree and the branch removed, with all that is in them). The decision is recorded on the run (`worktree.outcome`). A commit leaves out what is in the worktree's `.harnix/` but the project's own flows (`.harnix/flows/`): the rest is the engine's (the worktrees it makes for its sub-agents are in the repository's `.wkts/`, which the engine adds to the root `.gitignore`); Harnix does not change the project's own `.gitignore`, so whether `.harnix/` is ignored is the project's call. |
| **Executor** | Who does it: `session` (a session working from a skill), `agent` (a sub-agent the main Agent hands it to), `program` (the application's code), `external` (a person or an outside agent claims it), `subflow` (another flow). |
| **Main session / main Agent** | Every run has one main session: the flow's main Agent. Session stations without a `scene`, and every sub-agent station, take their turns in it. The flow's `main` says where it goes, its model, and how hard the model reasons (`effort`: `low`, `medium`, `high`, `xhigh` or `max`; the settings' when absent). Select the run in the flow space's Runs tab and it is the conversation in the middle; you can talk to it. |
| **Sub-agent** (`kind: agent`) | A sub-agent of the main session: the main Agent hands the work item to it with its `Agent` tool (`agent_type`, `model` optional) and hands the result in. |
| **Session of its own** | A session station that names a `scene` has its own session (`session: new`, or `{key: …}` to share by key), which may go to another instance, a template's instance or a space, on another model, with an `effort` of its own. Stations sharing a session by key must open it alike (scene, contract, model, effort, place): the run keeps what the session was opened with. |
| **Lease** | Claiming a work item takes a lease for a while, renewed as the work goes on; when it runs out the work item is ready again (or retried by `retry`). |
| **Rework** | Send the run back to a station (`rework_to` says where it may go; `on_unmet` does it automatically), optionally only some of its work items. |
| **Pause** | What is in hand finishes: a running turn is not cut off and its result is recorded; nothing new is entered or dispatched; the run goes on when resumed. See "A run's states" below. |

## 3. Constraints and checks

Each point in time has one place that can stop work:

| | What it is | When it acts | Does it stop work |
|---|---|---|---|
| **Rule** | named prose: what must hold across the flow | told to the sessions that work | no |
| **Policy / permission** | `allow` / `ask` / `deny`: which tools, on which arguments | while work is done, every tool call | **yes** |
| **Gate** | a named deterministic check | when a station is judged, as an exit condition | **yes** |
| **A person's decision** | someone decides | at the exit (`confirm`), or a call held for them (`ask`) | **yes** |

A skill says how to do a station's work, a rule what must not be broken anywhere in the flow; both are only read by the Agent. Prose never widens or narrows what may be done — only permissions do. A rule that must be checked is written as a gate.

| Object | What it is |
|---|---|
| **Rule** | `rules/<name>.md`, its first `# ` heading its title. Flows and stations name them in `rules`; a station works under the flow's and its own, told to its session work items (before the skill), pinned when the station is entered. Never enforced. |
| **Policy** | `policies/<name>.yaml`: a named set of permissions, which flows and stations name in `policies`. |
| **Permission** | An `effect` (`allow` / `ask` / `deny`) and `tools` (names, or `ns__*` for a namespace), optionally `paths` (the call's path, glob; relative ones from the run's workspace), `commands` (the command, glob), `args` (input fields → globs), `message`. Patterns may hold references: `${run}`, `${inputs.k}`, `${item…}`, `${workspace}` (a task's project, or its worktree for one run in a worktree; a run of no project, its own directory), `${output_dir}`. |
| **Judging a call** | A work item works under the flow's policies, the flow's permissions, the station's policies, the station's permissions — references resolved when it is dispatched. For each tool call: of the permissions that match, `deny` over `ask` over `allow`; what becomes of a call none matches, and of one that touches a path outside the project, is the flow's `guard` (next row). Paths are normalised first (`..` and the like folded), and a `deny` with paths refuses when any of them matches; commands are split at `;` `&&` `|`, and an `allow` never passes a line with command substitution or redirection. `ask` is left to a person in the conversation, and the run waits for `approval`; unanswered within the instance's time for an answer (300 seconds by default), the engine takes it as a refusal. The main session and sessions of their own are answered the same way, by the flow. |
| **Guard** (`guard`) | `auto` (the default) or `strict`, at the top of the flow. `auto`: no approvals — a guarded tool (Bash, Write, Edit…) no permission names is allowed inside the project (or worktree); a path outside it is always refused, with a reason the Agent reads, and the run does not wait; an `allow` whose `paths` name a place outside opens it. `strict`: a guarded tool nothing allows is refused, and a path outside the project is asked of a person. Under both, the author's `ask`, `confirm` and `hold` wait for a person; what the engine asks about on its own (a write outside the project, `.git`, `.harnix`…) is answered by the flow by the same rules under `auto`, and left to a person under `strict`. For a run of no project, its own directory is the project. The project's `.harnix/` counts as outside it under both: it is the application's (its settings, rules, the project's own flows), and only an `allow` whose `paths` point into it opens it — one over `${workspace}/**` does not; the `state/scratchpad/` and `state/memory/` the engine lets a session write as its own are the exception, judged as any path in the project. A flow saved into a project in the authoring conversation is written by `flow_create`, never by a task's Write. |
| **Gate** | `gates/<name>.yaml`, named by an exit `{kind: gate, gate: <name>}`. Three kinds: `tool` (call a read-only application tool and assert on its result, judged on the spot), `command` (run a command in the run's workspace, passing on exit status 0; only where the application turned command gates on; it keeps only the basics of the environment, and the run reaches it there — `FLOW_RUN`, `FLOW_STATION`, `FLOW_WORKSPACE` (the project or worktree), `FLOW_CONTEXT_FILE` (the station's `context.json` in the process directory; and `FLOW_CONTEXT` when small) — never in the command line; its output is kept as the station's `gate-<gate>.log`; killed as a group when out of time), `host` (the application's own check, by name). |
| **A person's decision** | The exit `confirm` (`who` is handed to the application's authorizer); a permission's `ask`, waiting for a person; `hold: true`, waiting for someone to release the station; and a person may force a station through (with a reason, recorded) or send work back. Who may do which is the application's to decide — the flow hands it `who`. A task's sessions have no tool to ask questions with, so the model never stops to ask a person: where someone must look or decide, the author writes a `confirm` station. |

## 4. Interfaces to the outside

A flow meets other systems in three places: progress goes out, work comes in, and the application's own capabilities are used.

| Object | What it is |
|---|---|
| **Hook** | `hooks/<name>.yaml`: `on` (event kinds, `station.*` allowed; absent: every event), `flows` (only these; absent: every flow), `kind` (`webhook` for now), `action` (the URL), `secret_env` (optional: signed). **No flow names a hook**: put in the pack, it applies, and no flow changes. A failed delivery is tried three times more; it never holds a run up. |
| **External work item** (`kind: external`) | A work item waiting to be claimed from outside: a person or another system claims it with `flow.claim` (a lease for a while, renewed by `heartbeat`) and hands it in with `flow.submit`, checked against the output contract. `who` is handed to the application's authorizer. |
| **The application's capabilities** | What the application registers for flows: programs (`kind: program`, its own code), tools (what `tool` gates call), checks (`host` gates), templates and instances (where a session of its own goes). A flow uses only names in the catalog. |
| **Subflow** (`kind: subflow`) | A work item that starts a run of another flow and is handed in when that run ends; the exit `children` waits for every child run. |

## 5. Writing and trying

| Object | What it is |
|---|---|
| **The pack and its parts** | A pack holds flows and five kinds of part: `skills/<name>/SKILL.md`, `rules/<name>.md`, `policies/<name>.yaml`, `gates/<name>.yaml`, `hooks/<name>.yaml`. Each comes from somewhere: `builtin` (shipped), `override` (saved over a shipped one; deleting it brings the shipped one back), `user` (only saved), `project` (the project's own, below), `plugin` (a plugin's, read-only, with the plugin's name; only where an application embedding the library mounts plugins — saved again it is an `override`). |
| **A project's own flows** | A project may bring a pack of its own, committed with it: `<project>/.harnix/flows/<name>.yaml`, the parts under `skills/`, `rules/`, `policies/`, `gates/`, `hooks/` beside them. It counts only for tasks of that project, as the top layer: a flow or part of the same name is the project's there; a project's flow may use the shipped and saved parts, and a shipped or saved flow run in the project uses the project's part of a name it has. **Read only if the project is trusted**: until then its flows are listed but not read and cannot be started, with the reason `untrusted` ("trust the project first"), and a shipped flow of the same name is hidden in that project; once trusted, the next listing offers them. An edit needs no restart: the next listing and the next run read it. `flow.defs {project}` lists every flow a project's task may run. In the authoring conversation, ask the Agent to save "to this project" (`flow_create` with `project_root`); from then on that conversation saves in the project, and the runs it starts are that project's tasks. |
| **Flow conversation / general conversation** | Each flow has a conversation of its own: ask the Agent there to change it, dry-run it, start it, act on its runs — each done straight away, without asking you; validation still holds, and a definition with problems is not saved. With no flow selected it is the general conversation, for new flows and questions. |
| **Validation** | Before a definition is saved it is checked with the parts it uses, and not saved with problems; a part is checked when it is saved, and the flows it breaks are listed. |
| **Dry run** | The real kernel, everything outside mocked: no session, no program, no cost. See the path a run takes and the message each session station would get. |

## A run's states

| State | Meaning |
|---|---|
| running | some work item is being worked on |
| waiting | not over, nothing in hand: it waits on a person or the outside — an approval (a call held for a person), a confirmation, a hold, an unmet station, a failed work item, an external claim, child runs |
| pausing | paused, what was in hand still finishing |
| paused | nothing in hand; nothing new is entered or dispatched |
| finished / cancelled | over |

A pause lets what is in hand finish: a turn that is running is not cut off, its result is recorded, and the run goes on when resumed. Each station's time and tokens are recorded (session and sub-agent stations have tokens).

## How to define a flow

1. Decide the **stations** and their order: what can run side by side (`needs` the same station), where paths join (`needs: [a, b]`).
2. Give each station its **work** and **exit conditions**. For a session: a `skill` and an `output` contract; for a person's decision: `confirm`.
3. Put the main Agent's place and model in the flow's `main:`; a station on another model is a `kind: agent` or a session of its own.
4. Write **permissions**: what the station may touch. What several flows share goes in a policy in the pack, named in `policies`.
5. What must not be broken is a **rule**; what can be checked mechanically is a **gate**, in the exit conditions.
6. For a system that must know the progress, put a **hook** in the pack.
7. Validate (`flow_validate`), dry-run (`flow_dry_run`), then start.

## Examples

### In sequence: draft, then a person confirms

```yaml
name: draft-and-sign
title: Draft and sign
stations:
  - id: draft
    executor: {kind: session}        # no scene: in the main session
    skill: draft
    output:
      type: object
      required: [summary]
      properties: {summary: {type: string}}
    exit: [{kind: output}]
  - id: sign
    exit: [{kind: confirm}]
    rework_to: [draft]
```

### In a project: change the code, write a report, no approvals

```yaml
name: fix-and-report
title: Fix and report
# no guard: auto — free inside the project, nothing outside it
output_dir: "reports/${flow}-${date}${seq}"   # absent: flows/${flow}-${date}${seq}
stations:
  - id: fix
    executor: {kind: session}
    skill: fix                     # the skill may say "write the report to ${output_dir}/report.md"
    output: {type: object, required: [changed], properties: {changed: {type: array}}}
    exit: [{kind: output}, {kind: gate, gate: ci}]
  - id: review
    exit: [{kind: confirm}]        # where a person must look, a confirm station: a task never stops to ask
```

### Side by side, then joined

```yaml
name: review-both
stations:
  - {id: collect, executor: {kind: session}, skill: collect, output: {type: object}, exit: [{kind: output}]}
  - {id: code, needs: [collect], executor: {kind: agent, agent_type: reviewer}, skill: review-code, output: {type: object}, exit: [{kind: output}]}
  - {id: docs, needs: [collect], executor: {kind: agent}, skill: review-docs, output: {type: object}, exit: [{kind: output}]}
  - {id: join, needs: [code, docs], exit: [{kind: confirm}]}
```

### One work item per entry

```yaml
name: per-module
stations:
  - id: list
    executor: {kind: session}
    skill: list-modules
    output: {type: object, required: [modules], properties: {modules: {type: array}}}
    exit: [{kind: output}]
  - id: analyze
    foreach: ${stations.list.output.modules}
    max_parallel: 2
    executor: {kind: agent, model: claude-haiku-5-5}
    skill: analyze
    output: {type: object}
    exit: [{kind: output}]
```

### Rules, policies, gates

```yaml
name: guarded-draft
rules: [no-secrets]                 # every station keeps it
stations:
  - id: draft
    executor: {kind: session}
    skill: draft
    permissions:                    # this station writes drafts; the release folder needs a nod
      - {effect: allow, tools: [Write, Edit], paths: ["${workspace}/drafts/**"]}
      - {effect: ask, tools: [Write], paths: ["${workspace}/release/**"], message: writing a release needs a person's yes}
    output: {type: object}
    exit: [{kind: output}, {kind: gate, gate: ci}]
  - id: review
    executor: {kind: agent}
    skill: review-docs
    policies: [read-only]           # a review only looks
    output: {type: object}
    exit: [{kind: output}]
```

`deny` always wins: what the flow refuses, no station opens. So a general refusal goes on the stations it is for, and what is opened is written where it is needed.

The parts beside it:

```yaml
# rules/no-secrets.md is markdown:
#   # No secrets
#   Keys, passwords, tokens never go into an output.

# policies/read-only.yaml
name: read-only
permissions:
  - {effect: deny, tools: [Write, Edit, NotebookEdit], message: this station looks and changes nothing}
```

```yaml
# gates/ci.yaml — the tests, run in the run's workspace
name: ci
title: Tests pass
kind: command
command: cargo test --quiet
timeout_seconds: 600
```

```yaml
# hooks/erp.yaml — progress to the ERP, no flow changed
name: erp
on: [run.created, "station.*", task.asking, run.finished, run.cancelled]
flows: [guarded-draft]
kind: webhook
action: https://erp.example.com/harnix/flow
secret_env: ERP_HOOK_SECRET
```

A hook is sent a `POST` of JSON — `{hook, delivery, event: {kind, run, flow, at, station?, task?, data}, run: {id, flow, title, subject, status, phase, usage}}` — with `X-Harnix-Event`, `X-Harnix-Delivery` and `X-Harnix-Timestamp` headers, and `X-Harnix-Signature: sha256=<HMAC of "timestamp.body">` when `secret_env` is set — refuse an old timestamp. A delivery tried again keeps its `delivery`.

### A session of its own, elsewhere, on another model

```yaml
  - id: build
    executor: {kind: session, scene: flow-harnix, model: claude-sonnet-5-5, session: new}
    skill: build
    permissions: [{effect: allow, tools: [Bash, Write, Edit]}]
    output: {type: object}
    exit: [{kind: output}, {kind: gate, gate: ci}]
```

## Where it is used

- **In a project**: "New task" in the project's "+" menu — pick a flow, fill in the form its `inputs` make, write the subject, and it starts once confirmed. `/flow <flow> [subject]` in a conversation of the project does the same and leaves a card in the conversation linking to the task; with the subject given and nothing else to fill in it starts at once, otherwise the same dialog opens. The task's main session opens in the middle of the projects space, where you can talk to it; the right column's "Flow" tab shows which station it is at, each station's output and gates. "Create a flow for this project" in the project's menu opens the general conversation, and the flows made and saved there go to the project's `.harnix/flows/` (the project must be trusted first).
- **In the flows space** (its entry at the bottom of the left column): the left column lists the general conversation and every flow, the right column's "Flows" and "Runs" tabs every flow and every run. Select a flow and the conversation in the middle is that flow's own — ask the Agent there to change it, dry-run it, start it, act on its runs (pause, resume, confirm, force, rework… done straight away when you ask for them); its details (the diagram, the definition, its skills / rules / policies / gates / hooks, the YAML) open in the panel beside the middle. Select a run and the middle is its main session; its sub-agents and background tasks are listed in the run's details, open any of them. An author's `ask` held for a person shows its approval card in the main session.
- Nothing selected: the general conversation, for new flows and questions. A flow created there makes that conversation its own.
