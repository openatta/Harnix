/** The system space's names and limits, in one place. */

/** The system space's id: the backend extension's space, and the one this package registers. */
export const SYSTEM = "system";

/** The points this package defines for others to fill. */
export const CONSOLE_TAB = "system.console.tab";
export const INSPECTOR_TAB = "system.inspector.tab";

export const LIVE = new Set(["running", "waiting", "pending"]);

/** While a turn runs nothing is pushed per model call: how often its row's figures are read then. */
export const RUNNING_USAGE_EVERY = 3000;

/** One `observability.usage` call's most. */
export const USAGE_BATCH = 200;
/** A sub-agent's transcript, read a page of this many messages at a time. */
export const SUB_PAGE = 30;
/** How many main Agents the all tab lists before "more". */
export const PAGE = 100;

/** How long the first press of "stop" waits for the second (DSH's `KILL_ARM_MS`). */
export const STOP_ARM_MS = 3000;

/** The most of a task's output this page keeps: the engine keeps the last 256 KiB. */
export const OUTPUT_KEPT = 256 * 1024;

/** How often the extension log is read again while it is live. */
export const EXTLOG_EVERY = 2000;
/** The most lines of the extension log the page keeps: the process keeps at most 500 an extension. */
export const EXTLOG_KEPT = 1000;
