/**
 * Reference and validation (component C, flow-design §14, §17): the definition file's fields, read
 * off the JSON Schema the module generates from its Rust types (the static layer); what this
 * application registered for definitions to name (the catalog, the dynamic layer); and a definition
 * checked against both and against the graph rules no schema can say (`validate`).
 */

import { useState } from "react";
import { useNest, useT } from "@harnix/react";

import { errorText, useMethod } from "./hooks";
import { catalogEntries, methodName, schemaRows } from "./model";
import type { Catalog, Problem } from "./types";

export interface FlowReferenceProps {
  module?: string;
  /** What the validation box starts with (the selected flow's YAML). */
  initialYaml?: string;
}

const CATALOG_KEYS = ["scenes", "steps", "gates", "tools", "actions", "skills"] as const;

export function FlowReference({ module = "flow", initialYaml = "" }: FlowReferenceProps) {
  const t = useT();
  const [of, setOf] = useState<"definition" | "run">("definition");
  const schema = useMethod<unknown>(module, "schema", { of });
  const catalog = useMethod<Catalog>(module, "catalog", {});
  const rows = schema.value ? schemaRows(schema.value) : [];

  return (
    <div className="hx-flow-panel hx-flow-reference" data-testid="flow-reference">
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

      {/* Another flow selected: start again from its YAML. */}
      <Validate key={initialYaml} module={module} initialYaml={initialYaml} />
    </div>
  );
}

function Validate({ module, initialYaml }: { module: string; initialYaml: string }) {
  const t = useT();
  const nest = useNest();
  const [yaml, setYaml] = useState(initialYaml);
  const [problems, setProblems] = useState<Problem[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const validate = async () => {
    setBusy(true);
    setError(null);
    try {
      const answer = await nest.call<Problem[] | { problems?: Problem[] }>(methodName(module, "validate"), { yaml });
      setProblems(Array.isArray(answer) ? answer : (answer?.problems ?? []));
    } catch (e) {
      setError(errorText(e));
      setProblems(null);
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="hx-flow-section" data-testid="flow-validate-box">
      <div className="hx-flow-section-head">{t("flowui.ref.validate")}</div>
      <textarea
        className="hx-flow-input hx-flow-mono"
        data-testid="flow-validate-yaml"
        aria-label={t("flowui.ref.yaml")}
        rows={12}
        spellCheck={false}
        value={yaml}
        onChange={(e) => {
          setYaml(e.target.value);
          setProblems(null);
        }}
      />
      <div className="hx-flow-actions">
        <button type="button" className="btn sm primary" data-testid="flow-validate" disabled={busy || !yaml.trim()} onClick={() => void validate()}>
          {t("flowui.ref.validateButton")}
        </button>
      </div>
      {error ? (
        <div className="hx-flow-error" role="alert">
          {error}
        </div>
      ) : null}
      {problems ? (
        problems.length === 0 ? (
          <p className="hx-flow-tone-ok" data-testid="flow-valid">
            {t("flowui.ref.valid")}
          </p>
        ) : (
          <ul className="hx-flow-list hx-flow-problems" data-testid="flow-problems">
            {problems.map((p, i) => (
              <li key={i} className="hx-flow-tone-error" data-testid="flow-problem">
                {p.station ? <code>{p.station}</code> : null} {p.path ? <code className="hx-flow-faint">{p.path}</code> : null} {p.message}
                {p.code ? <span className="hx-flow-faint"> ({p.code})</span> : null}
              </li>
            ))}
          </ul>
        )
      ) : null}
    </section>
  );
}
