/**
 * Reference (component C, flow-design §14, §17): what this application registered for definitions
 * to name (the catalog, the dynamic layer), and the definition file's fields, read off the JSON
 * Schema the module generates from its Rust types (the static layer). The reference tab puts the
 * guide to flows above it (xops-03 §6); a definition is checked in the conversation.
 */

import { useState } from "react";
import { useT } from "@harnix/react";

import { useMethod } from "./hooks";
import { catalogEntries, schemaRows } from "./model";
import type { Catalog } from "./types";

export interface FlowReferenceProps {
  module?: string;
}

const CATALOG_KEYS = ["scenes", "steps", "gates", "tools", "actions", "skills", "templates", "instances", "spaces", "models"] as const;

export function FlowReference({ module = "flow" }: FlowReferenceProps) {
  const t = useT();
  const [of, setOf] = useState<"definition" | "run">("definition");
  const schema = useMethod<unknown>(module, "schema", { of });
  const catalog = useMethod<Catalog>(module, "catalog", {});
  const rows = schema.value ? schemaRows(schema.value) : [];

  return (
    <div className="hx-flow-panel hx-flow-reference" data-testid="flow-reference">
      <section className="hx-flow-section" data-testid="flow-catalog">
        <div className="hx-flow-section-head">{t("flowui.ref.catalog")}</div>
        {catalog.error ? <div className="hx-flow-error">{catalog.error}</div> : null}
        {catalog.value
          ? CATALOG_KEYS.map((k) => {
              const entries = catalogEntries(catalog.value?.[k]);
              return (
                <div key={k} className="hx-flow-field" data-testid={`flow-catalog-${k}`}>
                  <div className="hx-flow-label">{t("flowui.ref.cat." + k)}</div>
                  <div className="hx-flow-value">
                    {entries.length === 0 ? (
                      <span className="hx-flow-faint">{t("flowui.def.none")}</span>
                    ) : (
                      entries.map((e) => (
                        <code key={e.name} className="hx-flow-chip" title={e.description || undefined}>
                          {e.name}
                        </code>
                      ))
                    )}
                  </div>
                </div>
              );
            })
          : null}
      </section>

      <section className="hx-flow-section">
        <div className="hx-flow-section-head">
          <span>{t("flowui.ref.fields")}</span>
          <span className="hx-flow-grow" />
          <div className="seg hx-flow-seg" role="tablist">
            {(["definition", "run"] as const).map((x) => (
              <button key={x} type="button" role="tab" aria-selected={of === x} className={of === x ? "on" : ""} data-testid={`flow-ref-${x}`} onClick={() => setOf(x)}>
                {t("flowui.ref.of." + x)}
              </button>
            ))}
          </div>
        </div>
        {schema.error ? <div className="hx-flow-error">{schema.error}</div> : null}
        {!schema.value && !schema.error ? <p className="hx-flow-hint">{t("flowui.loading")}</p> : null}
        <dl className="hx-flow-fields" data-testid="flow-ref-fields">
          {rows.map((r, i) => (
            <div key={`${r.path}#${i}`} className={`hx-flow-fieldrow hx-flow-depth-${Math.min(r.depth, 6)}`} data-path={r.path}>
              <dt>
                <code>{r.path}</code> <span className="hx-flow-faint">{r.type}</span>
                {r.required ? <span className="hx-flow-req"> {t("flowui.ref.required")}</span> : null}
              </dt>
              {r.description ? <dd>{r.description}</dd> : null}
            </div>
          ))}
        </dl>
      </section>
    </div>
  );
}
