/**
 * The SDK as one classic script, for a page that is not bundled: `<script src="harnix-app-sdk.js">`
 * puts it at `window.HarnixApp` (`HarnixApp.connect()`, `HarnixApp.createMockHost()`). A frame's
 * page is an opaque origin, so it loads classic scripts: a module script would need CORS.
 */

export * from "./index.ts";
export { createMockHost } from "./mock.ts";
