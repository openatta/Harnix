# Flow guide

A flow is a directed acyclic graph of **stations**: a **run** carries one **subject** (a code base, a requirement, a draft…) from the first station to the last. Each station has its **work** and its **exit conditions**; when they all hold, the run moves on. A flow definition is a YAML file; it says what is done and how it is judged — how to do the work well is in the **skills**.

The whole picture comes in five groups, read from what is done to how it is written:

1. **The skeleton** — what is done: flow, station, work and tasks, exit conditions, output contract.
2. **Running** — who does it, how it goes: run, executor, main session, sub-agent, session of its own, lease, rework, pause.
3. **Constraints and checks** — what can stop work: rules, policies and permissions, gates, a person's decision.
4. **Interfaces to the outside** — how it meets other systems: hooks (progress going out), external tasks (work claimed and handed in from outside), the application's own capabilities.
5. **Writing and trying** — how one is made: the pack and its parts, flow conversations, validation, dry runs.

## 1. The skeleton

A flow is a directed acyclic graph of stations; each station has its **work** and its **exit conditions**, and moves on when they all hold.

| Object | What it is |
|---|---|
| **Flow** | A definition: `name`, `title`, `inputs` (what a run is started with), `main` (where the main session goes), `rules` / `policies` / `permissions` (what every station works under), `stations`. Changing it does not affect runs already started: a run pins its definition, with the policies and gates it uses. |
| **Station** | The unit of judgement. `needs` orders stations (absent = the previous one in the list, `[]` = none); `when` skips it unless it holds; `hold: true` waits for someone to release it. |
| **Work** | What a station does, one of: `executor` (one task), `tasks` (several, each with an id), `foreach` + `executor` (one task per item of a list, `max_parallel` at once), or none (a checkpoint, judged on entry). |
| **Task** | One piece of work. Session, sub-agent and program tasks are dispatched by the flow; external tasks wait to be claimed; a subflow task starts a child run. |
| **Exit condition** | All must hold: `output` (every task's output satisfies the `output` contract), `gate` (a gate of the pack passes), `confirm` (someone confirms), `children` (every child run finished). |
| **Output contract** | A JSON Schema. What a session or sub-agent hands in must satisfy it; later stations refer to it as `${stations.<id>.output.<field>}`. |

## 2. Running

A **run** carries one subject through the graph; each task is done by an **executor**, and the main session is the flow's main Agent.

| Object | What it is |
|---|---|
| **Run** | One execution of a flow on one subject. Its states are below. |
| **Executor** | Who does it: `session` (a session working from a skill), `agent` (a sub-agent the main Agent hands it to), `program` (the application's code), `external` (a person or an outside agent claims it), `subflow` (another flow). |
| **Main session / main Agent** | Every run has one main session: the flow's main Agent. Session stations without a `scene`, and every sub-agent station, take their turns in it. Select the run in the flow space's Runs tab and it is the conversation in the middle; you can talk to it. |
| **Sub-agent** (`kind: agent`) | A sub-agent of the main session: the main Agent hands the task to it with its `Agent` tool (`agent_type`, `model` optional) and hands the result in. |
| **Session of its own** | A session station that names a `scene` has its own session (`session: new`, or `{key: …}` to share by key), which may go to another instance, a template's instance or a space, on another model. |
| **Lease** | Claiming a task takes a lease for a while, renewed as the work goes on; when it runs out the task is ready again (or retried by `retry`). |
| **Rework** | Send the run back to a station (`rework_to` says where it may go; `on_unmet` does it automatically), optionally only some of its tasks. |
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
| **Rule** | `rules/<name>.md`, its first `# ` heading its title. Flows and stations name them in `rules`; a station works under the flow's and its own, told to its session tasks (before the skill), pinned when the station is entered. Never enforced. |
| **Policy** | `policies/<name>.yaml`: a named set of permissions, which flows and stations name in `policies`. |
| **Permission** | An `effect` (`allow` / `ask` / `deny`) and `tools` (names, or `ns__*` for a namespace), optionally `paths` (the call's path, glob; relative ones from the run's workspace), `commands` (the command, glob), `args` (input fields → globs), `message`. Patterns may hold references: `${run}`, `${inputs.k}`, `${item…}`, `${workspace}`. |
| **Judging a call** | A task works under the flow's policies, the flow's permissions, the station's policies, the station's permissions — references resolved when it is dispatched. For each tool call: of the permissions that match, `deny` over `ask` over `allow`; none matching, a guarded tool (Bash, Write, Edit…) is refused and any other allowed. Paths are normalised first (`..` and the like folded), and a `deny` with paths refuses when any of them matches; commands are split at `;` `&&` `|`, and an `allow` never passes a line with command substitution or redirection; a call that would be allowed but touches a path outside the session's project is asked of a person, unless the `allow` that matched names that place in its own `paths`. `ask` is left to a person in the conversation, and the run waits for `approval`; unanswered within the instance's time for an answer (300 seconds by default), the engine takes it as a refusal. The main session and sessions of their own are answered the same way, by the flow. |
| **Gate** | `gates/<name>.yaml`, named by an exit `{kind: gate, gate: <name>}`. Three kinds: `tool` (call a read-only application tool and assert on its result, judged on the spot), `command` (run a command in the run's workspace, passing on exit status 0; only where the application turned command gates on; it keeps only the basics of the environment, and the run reaches it there — `FLOW_RUN`, `FLOW_STATION`, `FLOW_WORKSPACE`, `FLOW_CONTEXT_FILE` (and `FLOW_CONTEXT` when small) — never in the command line; killed as a group when out of time), `host` (the application's own check, by name). |
| **A person's decision** | The exit `confirm` (`who` is handed to the application's authorizer); a permission's `ask`, waiting for a person; `hold: true`, waiting for someone to release the station; and a person may force a station through (with a reason, recorded) or send work back. Who may do which is the application's to decide — the flow hands it `who`. |

## 4. Interfaces to the outside

A flow meets other systems in three places: progress goes out, work comes in, and the application's own capabilities are used.

| Object | What it is |
|---|---|
| **Hook** | `hooks/<name>.yaml`: `on` (event kinds, `station.*` allowed; absent: every event), `flows` (only these; absent: every flow), `kind` (`webhook` for now), `action` (the URL), `secret_env` (optional: signed). **No flow names a hook**: put in the pack, it applies, and no flow changes. A failed delivery is tried three times more; it never holds a run up. |
| **External task** (`kind: external`) | A task waiting to be claimed from outside: a person or another system claims it with `app.flow.claim` (a lease for a while, renewed by `heartbeat`) and hands it in with `app.flow.submit`, checked against the output contract. `who` is handed to the application's authorizer. |
| **The application's capabilities** | What the application registers for flows: programs (`kind: program`, its own code), tools (what `tool` gates call), checks (`host` gates), templates and instances (where a session of its own goes). A flow uses only names in the catalog. |
| **Subflow** (`kind: subflow`) | A task that starts a run of another flow and is handed in when that run ends; the exit `children` waits for every child run. |

## 5. Writing and trying

| Object | What it is |
|---|---|
| **The pack and its parts** | A pack holds flows and five kinds of part: `skills/<name>/SKILL.md`, `rules/<name>.md`, `policies/<name>.yaml`, `gates/<name>.yaml`, `hooks/<name>.yaml`. Each comes from somewhere: `builtin` (shipped), `override` (saved over a shipped one; deleting it brings the shipped one back), `user` (only saved). |
| **Flow conversation / general conversation** | Each flow has a conversation of its own: ask the Agent there to change it, dry-run it, start it, act on its runs — each change asked of you. With no flow selected it is the general conversation, for new flows and questions. |
| **Validation** | Before a definition is saved it is checked with the parts it uses, and not saved with problems; a part is checked when it is saved, and the flows it breaks are listed. |
| **Dry run** | The real kernel, everything outside mocked: no session, no program, no cost. See the path a run takes and the message each session station would get. |

## A run's states

| State | Meaning |
|---|---|
| running | some task is being worked on |
| waiting | not over, nothing in hand: it waits on a person or the outside — an approval (a call held for a person), a confirmation, a hold, an unmet station, a failed task, an external claim, child runs |
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

### Side by side, then joined

```yaml
name: review-both
stations:
  - {id: collect, executor: {kind: session}, skill: collect, output: {type: object}, exit: [{kind: output}]}
  - {id: code, needs: [collect], executor: {kind: agent, agent_type: reviewer}, skill: review-code, output: {type: object}, exit: [{kind: output}]}
  - {id: docs, needs: [collect], executor: {kind: agent}, skill: review-docs, output: {type: object}, exit: [{kind: output}]}
  - {id: join, needs: [code, docs], exit: [{kind: confirm}]}
```

### One task per item

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

## In the flow space

- **Flows** tab: select a flow and its details open in the second side panel (the diagram, the definition, its skills / rules / policies / gates / hooks, the YAML); the conversation in the middle is that flow's own — ask the Agent there to change it, dry-run it, start it, act on its runs (pause, resume, confirm, force, rework… each asked of you).
- **Runs** tab: select a run and its details open in the second side panel; the middle is its main session. Its sub-agents and tasks are listed in the details, open any of them. A call held for a person shows its approval card in the main session.
- Nothing selected: the general conversation, for new flows and questions. A flow created there makes that conversation its own.
