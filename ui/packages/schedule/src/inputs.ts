/**
 * A flow's `inputs`, as a task target's form: one box per property of the flow's schema (`flow.defs`'
 * `inputs`). Small on purpose — text, a number, a yes/no, a choice; anything else is JSON — since the
 * flow's own new-task dialog is the full one, and a draft from it arrives here already read.
 */

export type InputKind = "string" | "number" | "boolean" | "enum" | "json";

export interface InputField {
  key: string;
  kind: InputKind;
  required: boolean;
  description?: string;
  options?: string[];
  default?: unknown;
}

/** The fields of an `inputs` schema, in its order. */
export function inputFields(schema: unknown): InputField[] {
  if (!schema || typeof schema !== "object") return [];
  const s = schema as { properties?: Record<string, unknown>; required?: unknown };
  const required = new Set(Array.isArray(s.required) ? s.required.filter((x): x is string => typeof x === "string") : []);
  return Object.entries(s.properties ?? {}).map(([key, raw]) => {
    const p = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
    const field: InputField = { key, kind: "json", required: required.has(key) };
    if (typeof p.description === "string") field.description = p.description;
    if (p.default !== undefined) field.default = p.default;
    const type = Array.isArray(p.type) ? p.type.find((x) => x !== "null") : p.type;
    if (Array.isArray(p.enum) && p.enum.every((x) => typeof x === "string")) {
      field.kind = "enum";
      field.options = p.enum as string[];
    } else if (type === "string") field.kind = "string";
    else if (type === "number" || type === "integer") field.kind = "number";
    else if (type === "boolean") field.kind = "boolean";
    return field;
  });
}

/** A value as its box shows it. */
export function inputText(field: InputField, value: unknown): string {
  if (value === undefined || value === null) return "";
  switch (field.kind) {
    case "string":
    case "enum":
      return typeof value === "string" ? value : JSON.stringify(value);
    case "number":
      return typeof value === "number" ? String(value) : "";
    case "boolean":
      return value === true ? "true" : "false";
    case "json":
      return JSON.stringify(value);
  }
}

/** The boxes' texts for given values (a draft, a schedule's), each field's default where none. */
export function inputTexts(fields: InputField[], values: Record<string, unknown> | undefined): Record<string, string> {
  return Object.fromEntries(fields.map((f) => [f.key, inputText(f, values && f.key in values ? values[f.key] : f.default)]));
}

/**
 * The inputs from the texts: a value per field filled in, or the key of why it cannot be read. An
 * optional field left blank is not sent (the flow applies its default); a required one is an error.
 */
export function readInputs(fields: InputField[], texts: Record<string, string>): { inputs: Record<string, unknown>; errors: Record<string, string> } {
  const inputs: Record<string, unknown> = {};
  const errors: Record<string, string> = {};
  for (const f of fields) {
    const text = (texts[f.key] ?? "").trim();
    if (f.kind === "boolean") {
      if (text === "true") inputs[f.key] = true;
      else if (f.required || text === "false") inputs[f.key] = false;
      continue;
    }
    if (!text) {
      if (f.required) errors[f.key] = "schedule.err.required";
      continue;
    }
    if (f.kind === "number") {
      const n = Number(text);
      if (Number.isFinite(n)) inputs[f.key] = n;
      else errors[f.key] = "schedule.err.number";
    } else if (f.kind === "json") {
      try {
        inputs[f.key] = JSON.parse(text);
      } catch {
        errors[f.key] = "schedule.err.json";
      }
    } else inputs[f.key] = text;
  }
  return { inputs, errors };
}
