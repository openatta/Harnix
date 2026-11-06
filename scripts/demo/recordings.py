#!/usr/bin/env python3
"""Make the demo's recordings from its conversation script (xops-04 §8).

    scripts/demo/recordings.py <conversations.json> <template recording> <workspaces dir> <out dir>

Each conversation becomes one recording, named as the empty application names a flow session's
under --replay-dir: `flow-conversation-<flow>`, `flow-conversation` (the general one),
`flow-run-<run>` (a run's main session). Every model call answers one step: a text, a tool call,
or a text then a tool call; a turn's last step is a text. After each turn comes the call the
engine makes to note what to remember, answered `[]`.

The request side of each call is the template recording's first call (replay is not strict: what
the session asks is not compared), its blobs copied along.
"""

import json
import shutil
import sys
from pathlib import Path

SESSION = "DemoConversation000000"
TS = 1788402066758


def safe(run: str) -> str:
    return "".join(c if c.isalnum() or c in "-_" else "-" for c in run)


def name_of(conv: dict) -> str:
    kind = conv["kind"]
    if kind == "general":
        return "flow-conversation"
    if kind == "flow":
        return f"flow-conversation-{conv['flow']}"
    return f"flow-run-{conv['run']}"


def calls_of(steps: list) -> list:
    """Steps grouped into model calls: (text or None, tool step or None)."""
    calls, pending = [], None
    for s in steps:
        if "tool" in s:
            calls.append((pending, s))
            pending = None
        else:
            if pending is not None:
                calls.append((pending, None))
            pending = s["text"]
    if pending is not None:
        calls.append((pending, None))
    return calls


class Writer:
    def __init__(self, template: dict):
        self.t = template
        self.lines = [{
            "type": "recording", "version": 1, "name": SESSION, "session_id": SESSION,
            "created_at": TS, "engine_version": "0.1.0",
        }]
        self.seq = 100

    def next(self) -> int:
        self.seq += 1
        return self.seq

    def call(self, turn: int, step: int, text, tool, final: bool, tokens: int):
        call = self.next()
        rec = {
            "type": "call", "seq": call, "ts": TS, "session_id": SESSION, "turn": turn, "step": step,
            "provider": self.t["provider"], "api_type": self.t["api_type"], "params": self.t["params"],
            "system": self.t["system"], "tools": self.t["tools"], "messages": self.t["messages"],
        }
        if step == 0 and self.t.get("input_map"):
            rec["input_map"] = self.t["input_map"]
        self.lines.append(rec)
        self.lines.append({"type": "chunk", "seq": self.next(), "ts": TS, "call": call,
                           "chunk": {"kind": "thinking_signature", "signature": f"sig-{call}"}})
        if text is not None:
            self.lines.append({"type": "chunk", "seq": self.next(), "ts": TS, "call": call,
                               "chunk": {"kind": "content_block_start", "index": 1, "block": {"type": "text", "text": ""}}})
            self.lines.append({"type": "text_chunks", "seq0": self.next(), "ts0": TS, "call": call,
                               "texts": [text], "dt": []})
        if tool is not None:
            tid = f"call_{tool['tool']}_{call}"
            args = json.dumps(tool["input"], ensure_ascii=False)
            self.lines.append({"type": "tool_args_chunks", "seq0": self.next(), "ts0": TS, "call": call,
                               "id": tid, "args": [args], "dt": []})
            self.lines.append({"type": "chunk", "seq": self.next(), "ts": TS, "call": call,
                               "chunk": {"kind": "tool_use", "id": tid, "name": tool["tool"], "input": tool["input"]}})
        stop = "end_turn" if final else "tool_use"
        usage = {"input_tokens": tokens, "output_tokens": max(20, len(text or "") // 2 + 40)}
        self.lines.append({"type": "chunk", "seq": self.next(), "ts": TS, "call": call,
                           "chunk": {"kind": "end_turn", "stop_reason": stop, "usage": usage}})
        self.lines.append({"type": "end", "seq": self.next(), "ts": TS, "call": call, "outcome": {"status": "ok"},
                           "stop_reason": stop, "usage": usage, "duration_ms": 900})

    def text(self) -> str:
        return "".join(json.dumps(l, ensure_ascii=False, separators=(",", ":")) + "\n" for l in self.lines)


def substitute(v, ws: str):
    if isinstance(v, str):
        return v.replace("${ws}", ws)
    if isinstance(v, list):
        return [substitute(x, ws) for x in v]
    if isinstance(v, dict):
        return {k: substitute(x, ws) for k, x in v.items()}
    return v


def main():
    if len(sys.argv) != 5:
        sys.exit(__doc__)
    script, template_dir, workspaces, out = (Path(a) for a in sys.argv[1:])
    template = next(
        json.loads(l) for l in (template_dir / "calls.jsonl").open() if json.loads(l).get("type") == "call"
    )
    convs = json.loads(script.read_text())["conversations"]
    for conv in convs:
        ws = str(workspaces / safe(conv["run"])) if conv["kind"] == "run" else ""
        w = Writer(template)
        for turn, t in enumerate(conv["turns"]):
            steps = substitute(t["steps"], ws)
            grouped = calls_of(steps)
            for i, (text, tool) in enumerate(grouped):
                w.call(turn, i, text, tool, final=tool is None, tokens=6000 + 1500 * i + 400 * turn)
            # What the engine asks after a turn: what to remember.
            w.call(turn, len(grouped), "[]", None, final=True, tokens=900)
        dest = out / name_of(conv)
        if dest.exists():
            shutil.rmtree(dest)
        shutil.copytree(template_dir / "blobs", dest / "blobs")
        (dest / "calls.jsonl").write_text(w.text())
        print(f"{name_of(conv)}: {len(conv['turns'])} turns")


if __name__ == "__main__":
    main()
