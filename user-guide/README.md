# Harnix user guide

English | [简体中文](README.zh-CN.md)

This guide is for a person who uses the `harnix` program. You do not need to write code. If you build your own application on Harnix, read [docs/](../docs/app-development.md) instead.

Names in "quotes" are the words on the screen. The screen has three parts: the **left column** (sessions and projects), the **centre** (the conversation) and the **right column** (tabs: Guide, Context, Usage, Files, Preview, Changes, Flow, Output).

Contents: 1 Install and first run · 2 Models and credentials · 3 Projects and trust · 4 Conversations · 5 Tasks · 6 Schedules · 7 Settings · 8 The system space · 9 Extension packages · 10 When something goes wrong

---

## 1. Install and first run

**Install.** Open the GitHub Releases page of Harnix and download the file for your machine:

| Machine | File ends with |
|---|---|
| Mac, Apple chip | `aarch64-apple-darwin.tar.gz` |
| Mac, Intel chip | `x86_64-apple-darwin.tar.gz` |
| Linux, x86-64 | `x86_64-unknown-linux-gnu.tar.gz` |
| Linux, ARM | `aarch64-unknown-linux-gnu.tar.gz` |

The file is named `harnix-<version>-<target>.tar.gz`. A `.sha256` file sits beside it: compare it with the checksum of what you downloaded. Unpack it. Inside is one program, `harnix`. There is no Windows build and no desktop installer in this version.

**Start.** Run `harnix` in a terminal. Open <http://127.0.0.1:4080/> in your browser. That is all: no environment variable is needed. With no model set up, Harnix still starts and tells you so.

**The empty page and its three cards.** A new conversation shows a greeting, the input box, and three cards under it. Each card goes away when its step is done, or when you press "Don't show again" (cards: "不再显示" in Chinese).

1. **Model.** "Open model settings" takes you to Settings, "Models & credentials" (section 2). The card turns into a tick when a model is ready, and says which one new sessions use.
2. **Project.** A project is a folder the agent reads and writes in. "Create or choose a project…" opens the "Add project" dialog: type a name to make a new folder, or pick an existing one.
3. **Scene and permissions.** Says which scene a new conversation starts in and what the default permission mode means. If the `code` scene is offered (a coding agent that works in a project), the card has "New session in code…". "Choose scene and mode…" opens the new-session dialog. "Change the defaults…" opens Settings, "Permissions". Press "Got it" when you have read it.

The default scene is `harnix`. The `code` and `mini` scenes can be chosen in the "New session" dialog under "Scene".

---

## 2. Models and credentials

Open the avatar menu, then **Settings**, then **Models & credentials**.

- **Add a provider.** Press "Add a provider". Give it a name (for example `anthropic` or `deepseek`), choose the "API mode" ("Anthropic", "OpenAI-compatible" or "OpenAI Responses"), fill "Base URL", "API key" and "Default model". "Other models" lists more models, separated by commas; they appear in the model menu of the input box.
- **Default provider.** One provider is the default. Use "Make default" to change it. New sessions use it.
- **Keys stay private.** A key is saved on this machine, in the global settings file of the engine. It is never sent to the browser or written to logs. When you edit a provider, the key shows only as its last four characters ("key configured"). Leave the field empty to keep the stored key.
- **A key from an environment variable.** Instead of storing a key, type the name of an environment variable in "Or: the environment variable holding the key". The key then stays off the disk. If you set no provider but start Harnix with `ANTHROPIC_AUTH_TOKEN` (or `ANTHROPIC_API_KEY`, and optionally `ANTHROPIC_BASE_URL`), Harnix notices and says so on the page.
- **Reasoning effort.** "Model parameters" has "Reasoning effort" (how much work the model puts into a reply), "Thinking", the most output tokens and a "Fallback model" used while the main model is overloaded. They apply to every session.
- **Models by task.** In "Models by task" you can send some jobs to another provider and model: "Sub-agents", "Compaction", "Memory", "Session titles", "Web search" and "Reading fetched pages". A job you leave unset uses the default provider.
- **Switch model in a conversation.** The model menu of the input box switches to any model you listed.

---

## 3. Projects and trust

A project is a folder. Conversations in it can read and write its files. Add one from the "+" next to "Projects" in the left column, or from the first-run card.

**Trust.** A new project is not trusted. An untrusted project can only make things stricter: its own hooks, scripts, MCP servers, looser permissions, models, plugins and skills that run commands are held back and do nothing. Trust it only when you know where the folder came from, because those things run commands on your computer.

- **The banner.** When you open a conversation in an untrusted project that has such settings, a banner above the conversation says "This project is not trusted: its … are not in force." Press "Trust…" to read what would be switched on and confirm. "Don't show again" hides the banner.
- **From the project menu.** "Trust this project…" and later "Stop trusting this project". A trusted project shows "Trusted project". If your global settings trust it, you must change that there.
- **What was held back.** The right column tab "Context" lists the items trust is holding back ("Held back by trust").

**Project overview.** Select a project in the left column and the right column shows "Overview": its sessions, "Recent changes", the "Settings in force" (each marked global, scene or project) and its "Trust" state.

**待我处理 ("Waiting for you").** The overview has one list of everything waiting for you in the project: permission approvals ("Approval"), "Plan review", questions ("Question"), forms ("Form") and, in tasks, confirmations and holds. Press an item to go straight to it. When the list is empty it says "Nothing is waiting for you."

---

## 4. Conversations

**Start one.** "New session" (⌘K; Ctrl+K elsewhere) opens an empty page. The session is created when you send the first message. In a project, the "New in this project…" menu item starts one there.

**Send.** Type in the input box. `/` lists commands, `@` refers to a file. Enter sends; Shift+Enter makes a new line.

**Queue and steer.** While the agent is working, a message you send waits in a queue ("Queued 1"). You can withdraw it ("Withdraw") or edit it. "Deliver now" (steer) puts the message into the running turn at its next step instead of waiting for the end. In Settings, "General" → "Enter while a turn runs" chooses what Enter does while a turn is running: "Queue it" or "Steer the turn". If the turn ends first, a steered message comes back as "Not delivered".

**The model asks you.** Sometimes a card appears:

- **A question** ("The model asks"): choose an answer, or "Answer in your own words", then "Answer". "Don't answer" declines.
- **A plan** ("Plan to review"): read it, then "Approve and start", or "Request changes", write what should change, and "Send feedback".
- **A form** from an MCP server: fill it in.

**Permissions.** Before an action that needs approval, the conversation shows "Wants to use …" with "Allow once", "Allow for this session", "Always allow" (written to the project's local settings; in an untrusted project only for this session) and "Deny". With no answer in time the request is denied. You can answer from any device; the others see "Answered elsewhere".

**Turn rail.** A thin rail at the right edge of the conversation has one tick per turn (it appears from two turns on). Point at a tick to see the question and the start of the reply. Click to jump there; older history is loaded when needed. ⌥↑ and ⌥↓ (Alt+↑ and Alt+↓ elsewhere) go to the previous and next turn. The rail is hidden when the conversation is narrower than about 560 px; the keys still work.

**Feedback.** Under each finished reply: "Good answer" and "Bad answer". For a bad one you may pick a reason ("Inaccurate", "Not helpful", "Unsafe", "Too slow", "Other") and add a note. You can update or "Remove" it later. Feedback is kept in this session's log on your machine. The model never sees it and it is never uploaded. It is included when you export the session.

**Right column.**
- "Files": the project's files, read-only. "Show ignored" adds ignored files.
- "Preview": opens a file as text, code, Markdown or an image. A file link in a tool call opens it here.
- "Changes": which files each turn changed, with each file's diff. The card "Files changed: N" in the conversation opens it.
- "Context": what the next model call carries (system blocks, skills, hooks, MCP servers and their tools), and what trust holds back.
- "Usage": tokens, calls, cache hits and time of this session, with "Open the trajectory".
- "Output": the live output of a background shell or monitor the agent started, read-only, one tab for each.
- "Guide": lists the pages the column can open.
Open or close the column with the arrow at the corner of the conversation header, or with "Open or close the right column" (no key by default; you can give it one in "Keyboard shortcuts").

**Rewind ("Rewind to here").** On a message, choose "Rewind to here" (Chinese: "回到这里"). Harnix first shows what would change: the turns taken out, the files restored, and files changed outside the session since. Optionally tick "Fork first" to keep the removed turns in a fork. Then "Rewind". The prompt you took back is put into the input box. Only what the file tools wrote is put back; what shell commands did (git, installs) is not. You cannot rewind while a turn is running.

**Fork, archive, export, import.** The session menu ("Session actions" next to a session) has "Fork from here", "Archive", "Close" (frees the runtime, keeps the history), "Delete" and "Export". Export gives Markdown (to read) or a zip archive (the session, its sub-agents' sessions, images and entries). "Include the recorded model calls" adds the full prompts; use it only to investigate. To bring a session back, use "Import…" in the left column and choose a `.zip`. It becomes a new session; if its project is not on this machine, it has no project.

**The trajectory.** Above the conversation, the "View" switch has "Conversation" and "Trajectory". The trajectory is a table of every turn and every model call, with sub-agents and tool calls, and a timeline above it. You can fold turns, calls and sub-agents, drag across the timeline to zoom, search, and open one call to see its input, output and what changed from the last call. Feedback appears on the call it belongs to. "Open in the session" goes back to the conversation.

---

## 5. Tasks

A **task** is a conversation in a project that follows a **flow**: a plan made of stations. The agent moves your subject from the first station to the last. Flows come in four kinds: built in, your own ("自建" / "User"), changed copies, and ones that live in a project ("项目自带" / "Project").

**New task.** In the left column, open the "+" menu of the project and choose "New task". The dialog asks, in order:

1. The flow (its source is shown).
2. The inputs that flow wants (a form).
3. The subject: what the task is about.
4. A title (optional).
5. "Output directory". It shows the real path. Empty means `flows/<flow>-<YYYYMMDD><NN>/` in the project: the date, then the number of today's run, starting at `01`. You can type another path, but it must be inside the project. If the folder already has files, you get a warning and may go on.
6. "Run in a worktree" (optional): see below.

Confirm, and the task starts at once. The dialog says plainly that the flow changes project files directly.

**No approvals by default.** A flow has a setting `guard`. The default is `auto`: inside the project, the agent's tools run without asking. A path outside the project (or the worktree) is refused, and the agent sees why. The flow's author may still write steps that wait for you (a confirmation, or a hold). `strict` makes the flow ask for more.

**The task in the project tree.** Tasks are listed under the project beside the sessions. A row shows the flow name, its state and its progress ("station 2/5"). A task started by a schedule is marked.

**Open a task.** The centre is the task's main conversation: you may speak into it, and you may switch the "View" to "Trajectory" (the turn rail marks where each station starts). The right column has the "Flow" tab: the flow graph (the current station highlighted), each station's state, time, tokens, output and gate results, and a link to the output directory ("Copy path", "Open the files tab"). Actions: "Pause", "Resume", "Cancel run", rework ("Rework to…"), and, where the flow asks for it, "Confirm", "Reject" and "Release". A station whose gate failed shows each reason; send the work back, or release it with a written reason. A command gate's full output is in the process directory.

**Two folders.** The **output directory** is for you: the deliverables. Harnix never ignores it, so commit what you want to keep. The **process directory** is `.harnix/runs/<run>/` in the project: the software's own notes (a task snapshot, the pinned flow, each station's output, gate logs). The first task writes `.harnix/.gitignore` so `runs/` and `worktrees/` stay out of git.

**Worktree tasks.** With "Run in a worktree" the task works on its own branch `harnix/<run>` in `.harnix/worktrees/<run>`, and your project is not touched until you say so. It needs a git repository with at least one commit. If another task is running in the project, the dialog says so. When the task ends the right column asks: "Merge" (into the project's current branch; what was left uncommitted is committed on the branch first, and on a conflict nothing in the project changes), "Keep the branch" (the default), or "Discard" (removes the worktree and the branch; this cannot be undone).

**Flows that live in a project.** Put a flow in `.harnix/flows/<name>.yaml` (its skills, rules, policies, gates and hooks go in folders beside it) and commit it with the project. It is only read when the project is trusted; before that the list says "trust the project first" ("信任项目后可用"). If a name exists twice, the project's flow wins and the list shows the source.

**Write a flow in conversation.** Open the system space, the "Flows" tab, and talk to the agent in the flow conversation: ask it to write, change, try or start a flow. It does this directly, with no approvals. A flow that does not validate cannot be saved. Select a flow to get a conversation of its own.

**`/flow`.** <!-- T-F --> In an ordinary conversation, type `/flow <name> [subject]`. A new task starts, and the conversation keeps a link card to it.

---

## 6. Schedules

<!-- T-SC2 -->
A schedule starts work at a time you choose. Harnix itself keeps time, so schedules only run while Harnix is running. If Harnix was off at the time, it makes up the most recent missed time once when it starts; set "When it was missed" to "Skip it" to make up nothing.

**What a schedule can do.**
- Send a turn to a session that already exists. If the session is busy, the message waits in its queue.
- Start a new session, in a project or not: "New the first time, then the same one" (the default) or "A new one each time". You can pick its scene and model. A new session is named after the schedule, with the time.
- Start a task of a flow in a project, with its inputs, subject, task title, output directory and "Run in a worktree".

**When.** Pick one: "Once" (a date and time), "After…" (minutes, hours or days), "Every N" (minutes or hours), "Daily", "Weekly" (click the days), "Workdays", "Monthly" (a day of the month; months without that day are skipped) or "Custom" (a cron line of five fields: minute hour day month weekday). A line under it says the choice in words, such as "Workdays at 09:00". Pick the time zone from a list you can search by name, city or offset (`+8`); the default is your computer's, and zones you used lately come first. Next times are always shown in your own zone.

**Where to set one up.**
- A session's menu, **Schedule…**: that session's schedules, and **New schedule**, which sends it a turn.
- The new-task dialog and a flow's details in the system space, **Schedule this flow…**: what you filled in is carried over.
- A project's overview, the **Schedules** block: the project's schedules, and **New schedule** for a new session or a task in the project.
- The system space, the **Schedules** tab (below).

A session's row shows a clock while a schedule that is on sends to it. A task's row shows "next …" while the schedule that started it is on.

**Each schedule** shows its state ("On", "Paused", "Ended"), who made it, when it is next due (with "in 13 h"), and how the last delivery went. Its buttons: **Run now** (the result shows in a banner), **Pause** / **Resume**, **Edit**, **History** and **Delete** (after you confirm; the sessions and tasks it started stay). **History** lists the deliveries, newest first: the planned time, and delivered, skipped (with why) or failed (with the error). Each has a button that opens the session or task it went to. A failed delivery is not retried. History keeps the latest 200 of the last 30 days.

**The "Schedules" tab** lists every schedule. You can filter it by state and search titles and prompts. Schedules the model made itself are listed under "Made by models", with their cron line in UTC. You can only delete them here.

**If a task is still running** when the next time comes, a task schedule skips that time by default ("Skip this time"). Choose "Start another" to run in parallel; a worktree is better then. Two deliveries of one schedule are at least a minute apart, "Run now" too.

**If someone else changed it** while you were editing, saving says so. **Keep my changes and review** keeps what you changed, takes their version of the rest and names what they changed; check it and save again. **Take theirs** drops your changes.

---

## 7. Settings

Open the avatar menu and choose **Settings**. The left list has these sections.

- **General.** Theme (follow system, light, dark), language (also sets the language the model answers in), and "Display": font size, text font, code font, "Expand tool details", "Collapse thinking", "Token usage" (hidden, total only, total and cached), "Open links" (this tab, new tab, ask) and "Enter while a turn runs".
- **Keyboard shortcuts.** Every command with its key. Click a key, press the new combination (Esc gives up). A key needs Ctrl, ⌘ or ⌥, except function keys. If another command has it, you are told and nothing changes. "Restore default" and "Restore all defaults" are there. Changed keys are kept in this browser only.
- **Scenes.** The scenes and which are active ("Activate").
- **Models & credentials** and **Models by task.** Section 2.
- **Permissions, Sandbox, Network and web.** The default permission mode: "Ask (default)", "Accept edits in the project", "Plan: read only", "Don't ask: refuse what is not allowed", "Full access". Choosing "Full access" shows what it means and asks you to tick "I understand the risk": it asks nothing, even deletes. The "Permission rules" table lets you allow, ask about or deny a tool, or just some of its calls (a command such as `git push:*` for Bash, a path such as `src/**` for Read and Edit, a URL for WebFetch). The sandbox sets whether commands reach the network ("Unrestricted", "No network", "Allowed domains only", "Through the proxy only") and which more folders they may read or write. "Network and web" holds the proxy and the web-search settings.
- **MCP.** MCP servers are tools run on this machine, or services; add only what you trust. "Add a server": a name, "Transport" (a command with arguments, or a URL), "Environment" and "Headers". Values are saved in a local file with restricted access and later shown only as their last four characters. A server shows its state (connected, needs sign-in, failed, project not trusted…). For one that "needs sign-in", press "Sign in" and finish in the window that opens (allow pop-ups). Then "Sign out", "Reconnect" and "Switch off" are available.
- **Privacy.** "Record telemetry" is off by default in Harnix. Telemetry is the engine's own record of what it did: tokens and times, tool names, permission decisions. It holds no prompts or message text, and stays on your machine unless you give an export endpoint.
- **Advanced.** Read-only: what the engine runs with (compaction, execution, hooks, teams, checkpoints) and where each value comes from. To change one, edit the settings file; the engine reads it again.
- **Plugins**, **Extension packages** (section 9), **Devices**, **Diagnostics**, **About.**

**Where a setting is written.** Most fields have "Write to": "Global", the scene, or "This project". A value that is inherited says so. "Restore default" removes the value of that tier. If a project is not trusted, a value set there may be "set in this project but not in force". If the settings changed somewhere else while you edited, Harnix says so: "Keep my changes and review" shows your edits over the new values; then "Save my changes".

**Devices.** To use Harnix from another device, press "Pair a device…" and type the shown code on that device within the time limit. "Revoke" disconnects a device at once.

---

## 8. The system space

The entry is at the bottom of the left column ("System"). It is for looking at what Harnix is doing. The left column lists the engine **instances** with a state dot. The centre has a strip of figures (versions, uptime, memory, connected users, sessions running, how many wait for you) and tabs:

- **Active** and **All**: every main agent. Open a row for its sub-agents and background tasks; click it for details on the right. You can stop, interrupt, compact, fork, rename or delete from there, and send the agent a message.
- **Flows**: the flows and all their runs, including runs not in a project; where you also write flows in conversation (section 5).
- **Extensions**: the extensions and extension packages.
- **Extension log**: a short log of what extensions wrote; filter it by extension.
- **Usage**: what every session used in the last 30 days, by project, by model and by day, and by flow. A recording covers only a session's latest run.
- **Diagnostics**: also in Settings, "Diagnostics": which extension failed to start and why, and "Run doctor" for a self-check.
- **Schedules**: section 6. <!-- T-SC2 -->

The right column shows the picked item: its calls, turns (the tokens and time of each turn), context and background-task output. A session also has "Inspect in the system space" in its header to jump here. Raw frames are for developers.

---

## 9. Extension packages

An extension package (`.hxext`) adds something to Harnix: tools, a page, a skill. Open Settings, **Extension packages**.

- **Install from the store.** "Store" lists the packages that fit this Harnix. Press "Install…". If the store lists nothing, there is nothing for this Harnix. "No build for this Harnix" means that package is not made for your version.
- **Consent.** Before installing, Harnix shows who published it, how it is signed ("Countersigned by the store", "Signed by a trusted publisher" or "Unsigned (developer mode)") and a list "It will be able to:", for example reach a host on the network, read a project, store data, show its own page. If an update asks for more than the installed version, the new items are marked. Only install it if you agree.
- **Work with it.** Each package has "Enable" and "Disable", "Check for update" and "Update to …" (going back to an older version is possible and warns you), and "Clear data".
- **Remove.** "Uninstall". Its data is deleted with it.
- **If it fails.** A package that fails is shown with its reason and is set aside. Fix the cause, disable and enable it, and look at the "Extension log" in the system space.

---

## 10. When something goes wrong

Harnix says what happened and what to do. The engine's own words and the error code are in the hover text.

| What you see | What to do |
|---|---|
| No model service or key, when you send | A line with "Open model settings". Add a provider in "Models & credentials". |
| The service refuses the key, the account has no credit, or the model name is wrong | Check the key, the credit or the model name. The line has "Open model settings". |
| Rate limit, overloaded service, cannot connect, time out | Wait a little and send again (the message says how long when the service tells). If it cannot connect, check the network and the proxy. |
| The conversation is too long for the model | Compact the conversation, or start a new session. |
| The connection to the engine dropped, or an instance is unreachable | Wait and send again. If it does not come back, open Settings, "Diagnostics". |
| An extension package failed or did not start | Open Settings, "Extension packages": the package row gives the reason. Read the system space's "Extension log". Fix it, then disable and enable the package. A page of a package has "Reload" and "Try again". |
| A flow station's gate is not met | The station shows "unmet" with each reason. Send the work back, or release it with a written reason. A command gate's full output is in `.harnix/runs/<run>/stations/<station>/gate-*.log`. |
| A task path is outside the project | The step is refused and the agent is told. The flow's author can allow the path in the flow's permissions. |
| Settings changed somewhere else while you edited | "Keep my changes and review", then "Save my changes". |

Anything Harnix does not recognise is shown in the engine's own words.
