/**
 * An `EventSource` for node, on `fetch`.
 *
 * Node has no `EventSource` of its own on every version this runs on, and the
 * client takes one by injection anyway. `opened` resolves once the stream is
 * attached: a frame sent before that is a frame this reader never sees, so a
 * test waits for it before doing anything.
 */

import type { EventSourceLike } from "@harnix/core/client";

export class FetchEventSource implements EventSourceLike {
  onmessage: ((event: { data: string }) => void) | null = null;
  onerror: ((event: unknown) => void) | null = null;
  readonly opened: Promise<void>;
  private readonly abort = new AbortController();

  constructor(url: string) {
    let open!: () => void;
    this.opened = new Promise((resolve) => (open = resolve));
    void this.run(url, open);
  }

  private async run(url: string, open: () => void): Promise<void> {
    try {
      const response = await fetch(url, { signal: this.abort.signal });
      open();
      if (!response.ok || !response.body) {
        this.onerror?.(new Error(`HTTP ${response.status}`));
        return;
      }
      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";
      for (;;) {
        const { done, value } = await reader.read();
        if (done) return;
        buffer += decoder.decode(value, { stream: true });
        let cut: number;
        while ((cut = buffer.indexOf("\n\n")) !== -1) {
          const chunk = buffer.slice(0, cut);
          buffer = buffer.slice(cut + 2);
          const data = chunk
            .split("\n")
            .filter((line) => line.startsWith("data:"))
            .map((line) => line.slice(5).trim())
            .join("\n");
          if (data) this.onmessage?.({ data });
        }
      }
    } catch (e) {
      open();
      if (!this.abort.signal.aborted) this.onerror?.(e);
    }
  }

  close(): void {
    this.abort.abort();
  }
}
