# Harnix user guide

English | [简体中文](README.zh-CN.md)

This guide is for a person who uses the `harnix` program. You do not need to write code. If you build your own application on Harnix, read [docs/](../docs/app-development.md) instead.

Names in "quotes" are the words on the screen. The screen has three parts: the **left column** (Home, sessions and projects, with Flows and your avatar at the bottom), the **centre** (the conversation) and the **right column** (tabs: Guide, Context, Usage, Files, Preview, Changes, Flow, Output).

Contents: 1 Install and first run · 2 Models and credentials · 3 Projects and trust · 4 Conversations · 5 Tasks · 6 Schedules · 7 Settings · 8 The home page and flows · 9 Extension packages · 10 When something goes wrong

---

## 1. Install and first run

**Install.** Open the GitHub Releases page of Harnix and download the file for your machine:

| Machine | File ends with |
|---|---|
| Mac, Apple chip | `aarch64-apple-darwin.tar.gz` |
| Mac, Intel chip | `x86_64-apple-darwin.tar.gz` |
| Linux, x86-64 | `x86_64-unknown-linux-gnu.tar.gz` |
| Linux, ARM | `aarch64-unknown-linux-gnu.tar.gz` |

The file is named `harnix-<release>-<target>.tar.gz`, where `<release>` is the release's tag (`Harnix-v<x.y.z>-<YYYYMMDD><NN>`). Beside it is the same name with `.sha256` added: compare it with the checksum of what you downloaded (`shasum -a 256 <file>` on a Mac, `sha256sum <file>` on Linux). Unpack it. The folder inside holds the program `harnix`, with `LICENSE` and `README.md`; the interface is built into the program. There is no Windows build and no desktop installer in this version.

**Start.** Run `harnix` in a terminal. Open <http://127.0.0.1:4080/> in your browser. That is all: no environment variable is needed. To use another port, start it with `harnix --port <port>`. With no model set up, Harnix still starts and tells you so.

**The empty page and its three cards.** A new conversation shows a greeting, the input box, and three cards under it. A card whose step is already done is not shown; one you finish while the page is open turns into a tick. Close a card with its "Don't show again" button.

1. **Model.** "Open model settings" takes you to Settings, "Models & credentials" (section 2). Once a model is ready the card says which one new sessions use.
2. **Project.** A project is a folder the agent reads and writes in. "Create or choose a project…" opens the "Add project" dialog: type a name to make a new folder under the projects directory (`~/Documents` unless Harnix was started with another), or pick an existing one.
3. **Scene and permissions.** Says which scene a new conversation starts in and what the default permission mode means. If the `code` scene is offered (a coding agent that works in a project), the card has "New session in code…". "Choose scene and mode…" opens the new-session dialog. "Change the defaults…" opens Settings, "Permissions". Press "Got it" when you have read it.

The default scene is `harnix`, and `code` is offered beside it. The "New session" dialog lists the scenes under "Scene"; a scene that is not active yet says "(inactive)" (activate it in Settings, "Scenes").

---

## 2. Models and credentials

Click your avatar at the bottom of the left column, then **Settings**, then **Models & credentials**.

- **Add a provider.** Press "Add a provider". Give it a name (for example `anthropic` or `deepseek`), choose the "API mode" ("Anthropic", "OpenAI-compatible" or "OpenAI Responses"), fill "Base URL", "API key" and "Default model". "Other models" lists more models, separated by commas; they appear in the model menu of the input box.
- **Default provider.** One provider is the default. Use "Make default" to change it. New sessions use it.
- **Keys stay private.** A key is saved on this machine, in the global settings file of the engine. It is never sent to the browser or written to logs. When you edit a provider, the key shows only as its last four characters ("key configured"). Leave the field empty to keep the stored key.
- **A key from an environment variable.** Instead of storing a key, type the name of an environment variable in "Or: the environment variable holding the key". The key then stays off the disk. If you set no provider but start Harnix with `ANTHROPIC_AUTH_TOKEN` (or `ANTHROPIC_API_KEY`, and optionally `ANTHROPIC_BASE_URL`), Harnix notices and says so on the page.
- **Reasoning effort.** "Model parameters" has "Reasoning effort" (how much work the model puts into a reply), "Thinking", the most output tokens and a "Fallback model" used while the main model is overloaded. They apply to every session.
- **Models by task.** In "Models by task" you can send some jobs to another provider and model: "Sub-agents", "Compaction", "Memory", "Session titles", "Web search" and "Reading fetched pages". A job you leave unset uses the default provider.
- **Switch model in a conversation.** The model menu of the input box switches to any model you listed.

---

## 3. Projects and trust

A project is a folder. Conversations in it can read and write its files. Add one with the "+" beside "Projects" in the left column ("Add project"), or from the first-run card.

Each project row has two buttons. The "+" ("New in this project…") offers "New session" and "New task" (section 5). The "⋯" ("Project actions", also a right click on the project) has "New session in this project", "New task", "Create a flow for this project" (section 5), the trust items below, "Rename" and "Remove group" (forgets the grouping only; the sessions stay).

**Trust.** A new project is not trusted. An untrusted project can only make things stricter: its own hooks, scripts, MCP servers, looser permissions, models, plugins and skills that run commands are held back and do nothing. Trust it only when you know where the folder came from, because those things run commands on your computer.

- **The banner.** When you open a conversation in an untrusted project that has such settings, a banner above the conversation says "This project is not trusted: its … are not in force." Press "Trust…" to read what would be switched on and confirm. "Don't show again" hides the banner.
- **From the project menu.** "Trust this project…" and later "Stop trusting this project". A trusted project shows a shield ("Trusted project"). If your global settings trust it, you must change that there.
- **What was held back.** The right column tab "Context" lists the items trust is holding back ("Held back by trust").

**Project overview.** Open a project in the left column while no conversation is open, and the right column shows "Overview": "Waiting for you", its "Sessions", its "Tasks", its "Schedules", "Recent changes", the "Settings in force" (each marked global, scene or project) and its "Trust" state.

**"Waiting for you".** The overview has one list of everything waiting for you in the project: permission approvals ("Approval"), "Plan review", questions ("Question"), forms ("Form") and, in tasks, stations "To confirm", "Held" ones and an "External work item" to hand in. Press an item to go straight to it. When the list is empty it says "Nothing is waiting for you."

---

## 4. Conversations

**Start one.** "New session" (⌘K on a Mac, Ctrl+K elsewhere) opens an empty page. The session is created when you send the first message. In a project, the project's "+", then "New session", starts one there.

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
- "Context": always the last tab of every session. What the next model call carries (system blocks with their source and size, tools, skills, hooks, MCP servers and their tools), and what trust holds back. It reads again when a turn ends.
- "Usage": tokens, calls, cache hits and time of this session, with "Open the trajectory".
- "Output": the live output of a background shell command or monitor the agent started, read-only, one tab for each. Open it with "View output" on that tool call. The engine keeps only the last twenty finished ones.
- "Flow": in a task's conversation, the task (section 5).
- "Guide": lists the pages the column can open; the "+" ("Open a page…") at the top of the column opens it.

The button at the right end of the header above the conversation opens and closes the column ("Expand the right column" / "Collapse the right column"). At the top of the column, "Maximize" widens it and the × closes it. The command "Open or close the right column" has no key by default; you can give it one in "Keyboard shortcuts".

**Rewind ("Rewind to here").** On a message, choose "Rewind to here". Harnix first shows what would change: the turns taken out, the files restored, and files changed outside the session since. Optionally tick "Fork first" to keep the removed turns in a fork. Then "Rewind". The prompt you took back is put into the input box. Only what the file tools wrote is put back; what shell commands did (git, installs) is not. You cannot rewind while a turn is running.

**Fork, archive, export, import.** The session menu ("Session actions": the "⋯" beside a session, or the session's title above the conversation) has "Open", "Rename", "Fork from here", "Close" (frees the runtime, keeps the history), "Archive", "Export", "Schedule…" (section 6) and "Delete". Export gives Markdown (to read) or a zip archive (the session, its sub-agents' sessions, images and entries). "Include the recorded model calls" adds the full prompts; use it only to investigate. To bring a session back, use "Import…" in the left column and choose a `.zip`. It becomes a new session; if its project is not on this machine, it has no project.

**The trajectory.** Above the conversation, the "View" switch has "Conversation" and "Trajectory". The trajectory is a table of every turn and every model call, with sub-agents and tool calls, and a timeline above it. You can fold turns, calls and sub-agents, drag across the timeline to zoom, search, and open one call to see its input, output and what changed from the last call. Feedback appears on the call it belongs to. "Open in the session" goes back to the conversation.

---

## 5. Tasks

A **task** is a run of a **flow** in a project: a plan made of stations. The agent moves your subject from the first station to the last; the pieces of work a station hands out are its **work items**. Each flow shows where it comes from: "built in", "written here" (one you made), "changed" (a changed copy of a built-in one) or "this project's" (it lives in the project).

**New task.** In the left column, press the project's "+" and choose "New task" (it is also in the project's "⋯" menu and in the "Tasks" block of the overview). The dialog asks, in order:

1. "Flow": pick one (its source is shown; one that does not validate cannot be picked).
2. The inputs that flow wants (a form).
3. "Subject" (required): what the task is about.
4. "Title (optional)".
5. "Output directory", relative to the project. Empty means the flow's own directory if it names one, otherwise `flows/<flow>-<YYYYMMDD><NN>/`: the date, then the next free number of the day, starting at `01`. The field shows what empty gives. A path you type must stay inside the project.
6. "Run in a worktree" (optional): see below.

If another task is already running in the project, the dialog says so. Press "Start", and the task starts at once. Unless the flow is strict, the dialog says plainly that the flow changes project files directly. "Schedule this flow…" in the same dialog makes a schedule of it instead (section 6).

**No approvals by default.** A flow has a setting `guard`. The default is `auto`: inside the project, the agent's tools run without asking. A path outside the project (or the worktree) is refused, and the agent sees why. A task asks you no questions; the flow's author may still write stations that wait for you (a confirmation, or a hold). `strict` refuses what nothing allows and asks you about a path outside.

**The task in the project tree.** Tasks are listed under the project beside the sessions. A row shows the task's title, the flow, its state and its progress ("station 2/5"); a worktree task also shows its branch. A task started by a schedule has a clock.

**Open a task.** The centre is the task's main conversation: you may speak into it, and you may switch the "View" to "Trajectory" (the turn rail marks where each station starts). The right column opens on the "Flow" tab: the flow graph, a table of the stations ("Station", "State", "Time", "Tokens", "Output and gates"), and the output directory ("Copy path", "Open the files tab"). "Pause", "Resume" and "Cancel run" act on the whole task. Click a station, in the graph or the table, for its details and its actions: where the flow asks for them "Confirm" and "Reject", "Release" for a hold, "Rework to…" (send work back, optionally only some work items), "Force" (let it through by hand with a reason) and "Retry" for a failed work item. A station whose gate failed says "Still unmet" with each reason: send the work back, or force it through with a written reason. A command gate's full output is in the process directory.

**Two folders.** The **output directory** is for you: the deliverables. Harnix never ignores it, so commit what you want to keep. The **process directory** is `.harnix/runs/<run>/` in the project: the software's own notes (a task snapshot, the pinned flow, each station's output, gate logs). The first task writes `.harnix/.gitignore` so `runs/` and `worktrees/` stay out of git.

**Worktree tasks.** With "Run in a worktree" the task works on its own branch `harnix/<run>` in `.harnix/worktrees/<run>`, and your project is not touched until you say so. It needs a git repository with at least one commit. When the task is over, the "Flow" tab asks "The task is over: what becomes of its branch?": "Merge" (into the project's current branch; what was left uncommitted is committed on the branch first, and on a conflict nothing in the project changes), "Keep the branch", or "Discard" (removes the worktree and the branch; this cannot be undone). Until you choose, the branch stays.

**Flows that live in a project.** Put a flow in `.harnix/flows/<name>.yaml` (its skills, rules, policies, gates and hooks go in folders beside it) and commit it with the project. It is only read when the project is trusted; before that the list says "trust the project first". If a name exists twice, the project's flow wins and the list shows the source.

**Write a flow in conversation.** Flows are written by talking to the agent. There are two ways in:

- **For a project:** the project's "⋯" menu, "Create a flow for this project". If the project is not trusted, Harnix first offers to trust it, because a project's own flows are read only once it is trusted. Then the flows' general conversation opens, working in that project (what it saves goes to the project's `.harnix/flows/`), with a first line about the project in the input box.
- **In the Flows space** (section 8): the "Flows" tab lists the flows. "Add" opens the general conversation with "Create a new flow: " in the input box. Click a flow to work on it: the left column opens it on its own conversation with its runs under it, and its details open beside the conversation with "Dry run", "Start run" and "Schedule this flow…".

Ask the agent to write, change, try or start a flow. It does this directly, with no approvals. A flow that does not validate cannot be saved.

**`/flow`.** In a conversation of a project, type `/flow <flow> [subject]`; the `/` menu lists the flows the project can start. With a subject, and nothing the flow must still ask for, the task starts at once; otherwise the "New task" dialog opens on that flow with the subject filled in. `/flow` alone opens the dialog. The conversation stays as it was and keeps a card, "Task started with <flow>", showing the task's state and station, with "Open the task". In a conversation with no project, `/flow` says that a task belongs to a project and starts nothing.

---

## 6. Schedules

A schedule starts work at a time you choose. Harnix itself keeps time, so schedules only run while Harnix is running. If Harnix was off at the time, it makes up the most recent missed time once when it starts; set "When it was missed" to "Skip it" to make up nothing.

**What a schedule can do ("What it does").**
- "A turn in this session": send a turn to a session that already exists. If the session is busy, the message waits in its queue.
- "A new session", in a project or not: "New the first time, then the same one" (the default) or "A new one each time". You can pick its scene and model. A new session is named after the schedule, with the time.
- "A task": a task of a flow in a project, with its inputs, subject, task title, output directory and "Run in a worktree".

**When.** Pick one: "Once" (a date and time), "After…" (minutes, hours or days), "Every N" (minutes or hours), "Daily", "Weekly" (click the days), "Workdays", "Monthly" (a day of the month; months without that day are skipped) or "Custom" (a cron line of five fields: minute hour day month weekday). A line under it says the choice in words, such as "Workdays at 09:00". Pick the time zone from a list you can search by name, city or offset (`+8`); the default is your computer's, and zones you used lately come first. Next times are always shown in your own zone.

**Where to set one up.**
- A session's menu, **Schedule…**: that session's schedules, and **New schedule**, which sends it a turn.
- The new-task dialog, and a flow's details in the Flows space, **Schedule this flow…**: what you filled in is carried over.
- A project's overview, the **Schedules** block: the project's schedules, and **New schedule** for a new session or a task in the project.
- The home page, the **Schedules** block (below).

A session's row shows a clock while a schedule that is on sends to it. A task's row shows "next …" while the schedule that started it is on.

**Each schedule** shows its state ("On", "Paused", "Ended"), who made it ("Made by a person"), when and what it does, when it is next due (with "in 13 h") and how the last delivery went. Its buttons: **Run now** (the result shows in a banner), **Pause** / **Resume**, **Edit**, **History** and **Delete** (after you confirm; the sessions and tasks it started stay). **History** lists the deliveries, newest first: the planned time, and delivered, skipped (with why) or failed (with the error). Each has a button that opens the session or task it went to. A delivery says the work was handed over, not how it went. A failed delivery is not retried. History keeps the latest 200 of the last 30 days.

**The home page's "Schedules" block** lists every schedule, with **New schedule**. You can filter it by state and search titles and prompts. Schedules the model made itself are listed under "Made by models", with their cron line in UTC. You can only delete them here.

**If a task is still running** when the next time comes, a task schedule skips that time by default ("Skip this time"). Choose "Start another" to run in parallel; a worktree is better then. Two deliveries of one schedule are at least a minute apart, "Run now" too.

**If someone else changed it** while you were editing, saving says so. **Keep my changes and review** keeps what you changed, takes their version of the rest and names what they changed; check it and save again. **Take theirs** drops your changes.

---

## 7. Settings

Click your avatar at the bottom of the left column and choose **Settings**. The same menu has "Switch theme", "Language", "About" and "Connection". The command "Open the settings" has no key by default; you can give it one in "Keyboard shortcuts". The left list of Settings has these sections.

- **General.** Theme (follow system, light, dark), language (also sets the language the model answers in), and "Display": font size, text font, code font, "Expand tool details", "Collapse thinking", "Token usage" (hidden, total only, total and cached), "Open links" (this tab, new tab, ask) and "Enter while a turn runs".
- **Keyboard shortcuts.** Every command with its key. Click a key, press the new combination (Esc gives up). A key needs Ctrl, ⌘ or ⌥, except function keys. If another command has it, you are told and nothing changes. "Restore default" and "Restore all defaults" are there. Changed keys are kept in this browser only.
- **Scenes.** The scenes and which are active ("Activate").
- **Models & credentials** and **Models by task.** Section 2.
- **Permissions, Sandbox, Network and web.** The default permission mode: "Ask (default)", "Accept edits in the project", "Plan: read only", "Don't ask: refuse what is not allowed", "Full access". Choosing "Full access" shows what it means and asks you to tick "I understand the risk": it asks nothing, even deletes. The "Permission rules" table lets you allow, ask about or deny a tool, or just some of its calls (a command such as `git push:*` for Bash, a path such as `src/**` for Read and Edit, a URL for WebFetch). The sandbox sets whether commands reach the network ("Unrestricted", "No network", "Allowed domains only", "Through the proxy only") and which more folders they may read or write. "Network and web" holds the proxy and the web-search settings.
- **MCP.** MCP servers are tools run on this machine, or services; add only what you trust. "Add a server": a name, "Transport" (a command with arguments, or a URL), "Environment" and "Headers". Values are saved in a local file with restricted access and later shown only as their last four characters. A server shows its state (connected, needs sign-in, failed, project not trusted…). For one that "needs sign-in", press "Sign in" and finish in the window that opens (allow pop-ups). Then "Sign out", "Reconnect" and "Switch off" are available.
- **Privacy.** Telemetry is the engine's own record of what it did: tokens and times, tool names, permission decisions. It holds no prompts or message text, and stays on your machine unless you give an export endpoint. The main instance uses your own engine directory (`~/.agix` unless you chose another), so the engine's own default applies there, which is on, until you turn "Record telemetry" off here; that choice stays in that directory. Instances Harnix creates for itself start with it off.
- **Advanced.** Read-only: what the engine runs with (compaction, execution, hooks, teams, checkpoints) and where each value comes from. To change one, edit the settings file; the engine reads it again.
- **Plugins**, **Extension packages** (section 9), **Devices**, **About.**
- **Diagnostics.** The engine (version, uptime, sessions), which extension failed to start and why, and "Run doctor" for a self-check. Below them: "Packages as they run" (each extension package's state, calls, refused calls, a package process's last lines of errors; open a row for more) and the "Extension log": what each extension wrote, filtered by extension and by level, followed live, with "Copy".

**Where a setting is written.** Most fields have "Write to": "Global", the scene, or "This project". A value that is inherited says so. "Restore default" removes the value of that tier. If a project is not trusted, a value set there may be "set in this project but not in force". If the settings changed somewhere else while you edited, Harnix says so: "Keep my changes and review" shows your edits over the new values; then "Save my changes".

**Devices.** To use Harnix from another device, press "Pair a device…" and type the shown code on that device within the time limit. "Revoke" disconnects a device at once.

---

## 8. The home page and flows

**Home** is the first entry of the left column. A new tab, or Harnix started again, opens on it; reloading the page keeps you where you were. It holds no conversation: "New session" takes you to a space where you can talk. Its blocks:

- **At a glance**: the Harnix and Agix versions, uptime, peak memory, threads, connected users (point at it to see who), instances, agents active of all, sessions running, background tasks running, and how many things wait for you (click it for the list).
- **Waiting for you**: the questions and approvals waiting in every session, and the tasks waiting to be confirmed, released or claimed. Click one to go where you can answer it.
- **Recent sessions** and **Recent tasks**: the latest of each, with their project; click to open. "All sessions" goes to the sessions space.
- **Schedules**: every schedule, with **New schedule** (section 6).
- **Usage**: what every session used in the last 30 days, by project, by day and by model, and the flows by flow. A recording covers only a session's latest run.

**Flows** is at the bottom of the left column, above your avatar. It is where flows are managed. The left column has the general conversation (for writing new flows and asking questions) and every flow; click a flow to open its own conversation, with its runs and dry runs under it, and click a run to see its main conversation. The details of what you picked open beside the conversation; closing them keeps your pick. The right column has three tabs: "Flows" (filter by source and validity, search, sort), "Runs" (every run and dry run, including runs not in a project; filter and sort) and "Reference" (the flow guide). When you opened a task from a project, "Back to Projects" at the top of the left column takes you back there.

---

## 9. Extension packages

An extension package (`.hxext`) adds something to Harnix: tools, a page, a skill. Open Settings, **Extension packages**.

- **Install from the store.** "Store" lists the packages that fit this Harnix. Press "Install…". If the store lists nothing, there is nothing for this Harnix. "No build for this Harnix" means that package is not made for your version.
- **Consent.** Before installing, Harnix shows who published it, how it is signed ("Countersigned by the store", "Signed by a trusted publisher" or "Unsigned (developer mode)") and a list "It will be able to:", for example reach a host on the network, read a project, store data, show its own page. If an update asks for more than the installed version, the new items are marked. Only install it if you agree.
- **Work with it.** Each package has "Enable" and "Disable", "Check for update" and "Update to …" (going back to an older version is possible and warns you), and "Clear data".
- **Remove.** "Uninstall". Its data is deleted with it.
- **If it fails.** A package that fails is shown with its reason and is set aside. Fix the cause, disable and enable it, and look at Settings, "Diagnostics": "Packages as they run" and the "Extension log".

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
| An extension package failed or did not start | Open Settings, "Extension packages": the package row gives the reason. Read the "Extension log" in Settings, "Diagnostics". Fix it, then disable and enable the package. A page of a package has "Reload" and "Try again". |
| A flow station's gate is not met | The station shows "Still unmet" with each reason. Send the work back ("Rework to…"), or "Force" it through with a written reason. A command gate's full output is in `.harnix/runs/<run>/stations/<station>/gate-*.log`. |
| A task path is outside the project | The step is refused and the agent is told. The flow's author can allow the path in the flow's permissions. |
| Settings changed somewhere else while you edited | "Keep my changes and review", then "Save my changes". |

Anything Harnix does not recognise is shown in the engine's own words.
