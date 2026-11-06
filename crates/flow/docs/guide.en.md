# Flow guide

A flow is a directed acyclic graph of **stations**: a **run** carries one **subject** (a code base, a requirement, a draft…) from the first station to the last. Each station has its **work** and its **exit conditions**; when they all hold, the run moves on. A flow definition is a YAML file; it says what is done and how it is judged — how to do the work well is in the **skills**.

Besides flows and skills a pack holds four things: **rules**, **policies** (sets of **permissions**), **gates** and **hooks**. Each point in time has one place that can stop work:

| | What it is | When it acts | Does it stop work |
|---|---|---|---|
| **Rule** | named prose: what must hold across the flow | told to the sessions that work | no |
| **Policy / permission** | `allow` / `ask` / `deny`: which tools, on which arguments | while work is done, every tool call | **yes** |
| **Gate** | a named deterministic check | when a station is judged, as an exit condition | **yes** |
| **Hook** | tells a system outside about a run's events | after they happen | no |

A skill says how to do a station's work, a rule what must not be broken anywhere in the flow; both are only read by the Agent. Prose never widens or narrows what may be done — only permissions do. A rule that must be checked is written as a gate.

## The objects

| Object | What it is |
|---|---|
| **Flow** | A definition: `name`, `title`, `inputs` (what a run is started with), `main` (where the main session goes), `rules` / `policies` / `permissions` (what every station works under), `stations`. Changing it does not affect runs already started: a run pins its definition, with the policies and gates it uses. |
| **Station** | The unit of judgement. `needs` orders stations (absent = the previous one in the list, `[]` = none); `when` skips it unless it holds; `hold: true` waits for someone to release it. |
| **Work** | What a station does, one of: `executor` (one task), `tasks` (several, each with an id), `foreach` + `executor` (one task per item of a list, `max_parallel` at once), or none (a checkpoint, judged on entry). |
| **Task** | One piece of work. Session, sub-agent and program tasks are dispatched by the flow; external tasks wait to be claimed; a subflow task starts a child run. |
| **Executor** | Who does it: `session` (a session working from a skill), `agent` (a sub-agent the main Agent hands it to), `program` (the application's code), `external` (a person or an outside agent claims it), `subflow` (another flow). |
| **Exit condition** | All must hold: `output` (every task's output satisfies the `output` contract), `gate` (a gate of the pack passes), `confirm` (someone confirms), `children` (every child run finished). |
| **Output contract** | A JSON Schema. What a session or sub-agent hands in must satisfy it; later stations refer to it as `${stations.<id>.output.<field>}`. |
| **Run** | One execution of a flow on one subject. Its states are below. |
| **Main session / main Agent** | Every run has one main session: the flow's main Agent. Session stations without a `scene`, and every sub-agent station, take their turns in it. Select the run in the flow space's Runs tab and it is the conversation in the middle; you can talk to it. |
| **Sub-agent** (`kind: agent`) | A sub-agent of the main session: the main Agent hands the task to it with its `Agent` tool (`agent_type`, `model` optional) and hands the result in. |
| **Session of its own** | A session station that names a `scene` has its own session (`session: new`, or `{key: …}` to share by key), which may go to another instance, a template's instance or a space, on another model. |
| **Rule** | `rules/<name>.md`, its first `# ` heading its title. Flows and stations name them in `rules`; a station works under the flow's and its own, told to its session tasks (before the skill), pinned when the station is entered. Never enforced. |
| **Permission** | An `effect` (`allow` / `ask` / `deny`) and `tools` (names, or `ns__*` for a namespace), optionally `paths` (the call's path, glob; relative ones from the run's workspace), `commands` (the command, glob), `args` (input fields → globs), `message`. Patterns may hold references: `${run}`, `${inputs.k}`, `${item…}`, `${workspace}`. |
| **Policy** | `policies/<name>.yaml`: a named set of permissions, which flows and stations name in `policies`. |
| **Judging a call** | A task works under the flow's policies, the flow's permissions, the station's policies, the station's permissions — references resolved when it is dispatched. For each tool call: of the permissions that match, `deny` over `ask` over `allow`; none matching, a guarded tool (Bash, Write, Edit…) is refused and any other allowed. `ask` is left to a person in the conversation, and the run waits for `approval`. The main session and sessions of their own are answered the same way, by the flow. |
| **Gate** | `gates/<name>.yaml`, named by an exit `{kind: gate, gate: <name>}`. Three kinds: `tool` (call a read-only application tool and assert on its result, judged on the spot), `command` (run a command in the run's workspace, passing on exit status 0; only where the application turned command gates on; the run reaches it in the environment — `FLOW_RUN`, `FLOW_STATION`, `FLOW_WORKSPACE`, `FLOW_CONTEXT` — never in the command line), `host` (the application's own check, by name). |
| **Hook** | `hooks/<name>.yaml`: `on` (event kinds, `station.*` allowed; absent: every event), `flows` (only these; absent: every flow), `kind` (`webhook` for now), `action` (the URL), `secret_env` (optional: signed). **No flow names a hook**: put in the pack, it applies, and no flow changes. A failed delivery is tried three times more; it never holds a run up. |
| **Rework** | Send the run back to a station (`rework_to` says where it may go; `on_unmet` does it automatically), optionally only some of its tasks. |
| **Lease** | Claiming a task takes a lease for a while, renewed as the work goes on; when it runs out the task is ready again (or retried by `retry`). |
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

A hook is sent a `POST` of JSON — `{hook, delivery, event: {kind, run, flow, at, station?, task?, data}, run: {id, flow, title, subject, status, phase, usage}}` — with `X-Harnix-Event` and `X-Harnix-Delivery` headers, and `X-Harnix-Signature: sha256=<HMAC of the body>` when `secret_env` is set. A delivery tried again keeps its `delivery`.

### A session of its own, elsewhere, on another model

```yaml
  - id: build
    executor: {kind: session, scene: flow-coding, model: claude-sonnet-5-5, session: new}
    skill: build
    permissions: [{effect: allow, tools: [Bash, Write, Edit]}]
    output: {type: object}
    exit: [{kind: output}, {kind: gate, gate: ci}]
```

## In the flow space

- **Flows** tab: select a flow and its details open in the second side panel (the diagram, the definition, its skills / rules / policies / gates / hooks, the YAML); the conversation in the middle is that flow's own — ask the Agent there to change it, dry-run it, start it, act on its runs (pause, resume, confirm, force, rework… each asked of you).
- **Runs** tab: select a run and its details open in the second side panel; the middle is its main session. Its sub-agents and tasks are listed in the details, open any of them. A call held for a person shows its approval card in the main session.
- Nothing selected: the general conversation, for new flows and questions. A flow created there makes that conversation its own.
