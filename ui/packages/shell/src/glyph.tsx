/** An icon from `icons.ts`. The span carries the size: an SVG left to fill its row is a 300px icon. */

import { glyphHtml } from "./icons.ts";

export function Glyph({ name, className }: { name: string; className?: string }) {
  return <span className={className} aria-hidden="true" dangerouslySetInnerHTML={glyphHtml(name)} />;
}
