---
name: showcase-tour
description: A tour of the Showcase application's tools — use when asked what the showcase can do
---

# The Showcase tour

The Showcase application gives this session seven tools, all named `app__example_showcase__<tool>`:

1. `echo {text}` says the text back.
2. `count {key?}` counts one more and says the total; the count survives the call.
3. `config` shows the application's configuration.
4. `read_project {path}` reads a file of the current project (read-only).
5. `slow {steps?, ms?}` works for a while and reports each step.
6. `vetoed` is refused by the application's own event hook, every time.
7. `board` says what the application's backend holds: its notes and its counter.

Each one asks before it runs. Try `echo` first.
