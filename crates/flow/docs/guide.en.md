# Flow guide

A flow is a directed acyclic graph of **stations**: a **run** carries one **subject** (a code base, a requirement, a draft…) from the first station to the last. Each station has its **work** and its **exit conditions**; when they all hold, the run moves on. A flow definition is a YAML file; it says what is done and how it is judged — how to do the work well is in the **skills**.

## The objects

| Object | What it is |
|---|---|
| **Flow** | A definition: `name`, `title`, `inputs` (what a run is started with), `main` (where the main session goes), `stations`, `hooks`. Changing it does not affect runs already started: a run pins its definition. |
| **Station** | The unit of judgement. `needs` orders stations (absent = the previous one in the list, `[]` = none); `when` skips it unless it holds; `hold: true` waits for someone to release it. |
| **Work** | What a station does, one of: `executor` (one task), `tasks` (several, each with an id), `foreach` + `executor` (one task per item of a list, `max_parallel` at once), or none (a checkpoint, judged on entry). |
| **Task** | One piece of work. Session, sub-agent and program tasks are dispatched by the flow; external tasks wait to be claimed; a subflow task starts a child run. |
| **Executor** | Who does it: `session` (a session working from a skill), `agent` (a sub-agent the main Agent hands it to), `program` (the application's code), `external` (a person or an outside agent claims it), `subflow` (another flow). |
| **Exit condition** | All must hold: `output` (every task's output satisfies the `output` contract), `tool` (call an application tool and assert on the result), `gate` (a check the application runs), `confirm` (someone confirms), `children` (every child run finished). |
| **Output contract** | A JSON Schema. What a session or sub-agent hands in must satisfy it; later stations refer to it as `${stations.<id>.output.<field>}`. |
| **Run** | One execution of a flow on one subject. Its states are below. |
| **Main session / main Agent** | Every run has one main session: the flow's main Agent. Session stations without a `scene`, and every sub-agent station, take their turns in it. Select the run in the flow space's Runs tab and it is the conversation in the middle; you can talk to it. |
| **Sub-agent** (`kind: agent`) | A sub-agent of the main session: the main Agent hands the task to it with its `Agent` tool (`agent_type`, `model` optional) and hands the result in. |
| **Session of its own** | A session station that names a `scene` has its own session (`session: new`, or `{key: …}` to share by key), which may go to another instance, a template's instance or a space, on another model. |
| **Writes** | Guarded write tools (Bash, Write, Edit…) are refused unless a station's `writes` allow them. In the main session the flow answers each use from the station being worked on. |
| **Rework** | Send the run back to a station (`rework_to` says where it may go; `on_unmet` does it automatically), optionally only some of its tasks. |
| **Lease** | Claiming a task takes a lease for a while, renewed as the work goes on; when it runs out the task is ready again (or retried by `retry`). |
| **Dry run** | The real kernel, everything outside mocked: no session, no program, no cost. See the path a run takes and the message each session station would get. |
| **Hook** | Tells the application when something happens (`on: station.passed`, `action: notify`); it never holds a station up. |

## A run's states

| State | Meaning |
|---|---|
| running | some task is being worked on |
| waiting | not over, nothing in hand: it waits on a person or the outside — a confirmation, a hold, an unmet station, a failed task, an external claim, child runs |
| pausing | paused, what was in hand still finishing |
| paused | nothing in hand; nothing new is entered or dispatched |
| finished / cancelled | over |

A pause lets what is in hand finish: a turn that is running is not cut off, its result is recorded, and the run goes on when resumed. Each station's time and tokens are recorded (session and sub-agent stations have tokens).

## How to define a flow

1. Decide the **stations** and their order: what can run side by side (`needs` the same station), where paths join (`needs: [a, b]`).
2. Give each station its **work** and **exit conditions**. For a session: a `skill` and an `output` contract; for a person's decision: `confirm`.
3. Put the main Agent's place and model in the flow's `main:`; a station on another model is a `kind: agent` or a session of its own.
4. Write `writes`: what the station may change.
5. Validate (`flow_validate`), dry-run (`flow_dry_run`), then start.

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

### A session of its own, elsewhere, on another model

```yaml
  - id: build
    executor: {kind: session, scene: flow-coding, model: claude-sonnet-5-5, session: new}
    skill: build
    writes: [Bash, Write, Edit]
    output: {type: object}
    exit: [{kind: output}, {kind: gate, gate: ci}]
```

## In the flow space

- **Flows** tab: select a flow and the conversation in the middle is that flow's own — ask the Agent there to change it, dry-run it, start it, act on its runs (pause, resume, confirm, force, rework… each asked of you).
- **Runs** tab: select a run and the middle is its main session; its sub-agents and tasks are listed in the run's details, open any of them.
- Nothing selected: the general conversation, for new flows and questions. A flow created there makes that conversation its own.
