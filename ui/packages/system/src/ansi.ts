/** Terminal escape sequences: colours and cursor moves (CSI), and titles or links (OSC). */
const ESCAPES = /\u001b\[[0-?]*[ -/]*[@-~]|\u001b\][^\u0007\u001b]*(?:\u0007|\u001b\\)|\u001b[@-Z\\-_]/g;

/** The text of a command's output without its colour codes, which a page cannot draw. */
export function stripAnsi(text: string): string {
  return text.includes("\u001b") ? text.replace(ESCAPES, "") : text;
}
