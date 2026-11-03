/**
 * Form generated from a widget's {@link WidgetSchema}, plus the validation
 * and default-value helpers that go with it.
 */
import { useId, useState } from "react";
import type { FieldSchema, FieldValue, SecretPlaceholder, WidgetSchema } from "../../api/types";
import { isHttpUrl } from "./util";
import "./forms.css";

/** The value the server sends in place of a stored secret. */
export const SECRET_SET: SecretPlaceholder = Object.freeze({ $secret: "set" });

/** Whether a field value is the stored-secret placeholder. */
export function isSecretPlaceholder(value: FieldValue | undefined): value is SecretPlaceholder {
  return typeof value === "object" && value !== null && value.$secret === "set";
}

/** Whether a value counts as "not provided" for a required field. */
function isMissing(value: FieldValue | undefined): boolean {
  return (
    value === undefined || value === null || (typeof value === "string" && value.trim() === "")
  );
}

/**
 * Checks widget field values against their schema.
 *
 * Rules: required fields must be present (a stored secret placeholder counts
 * as present, `false` counts for booleans); `url` fields must be absolute
 * http(s) URLs; `number` fields must be numeric; `select` fields must hold
 * one of their options.
 *
 * @returns error messages keyed by field key; empty when everything is valid.
 */
export function validateWidget(
  schema: WidgetSchema,
  values: Record<string, FieldValue>,
): Record<string, string> {
  const errors: Record<string, string> = {};
  for (const f of schema.fields) {
    const v = values[f.key];
    if (isMissing(v)) {
      if (f.required) errors[f.key] = `${f.label} is required`;
      continue;
    }
    switch (f.kind) {
      case "url":
        if (typeof v !== "string" || !isHttpUrl(v)) {
          errors[f.key] = `${f.label} must start with http:// or https://`;
        }
        break;
      case "number":
        if (
          !(typeof v === "number" && Number.isFinite(v)) &&
          !(typeof v === "string" && Number.isFinite(Number(v)))
        ) {
          errors[f.key] = `${f.label} must be a number`;
        }
        break;
      case "select":
        if (f.options && !f.options.includes(String(v))) {
          errors[f.key] = `${f.label} must be one of ${f.options.join(", ")}`;
        }
        break;
      default:
        break;
    }
  }
  return errors;
}

/**
 * Initial values for a new widget of this schema: each field's `default`,
 * `false` for booleans, the first option for required selects, else `null`.
 */
export function defaultValues(schema: WidgetSchema): Record<string, FieldValue> {
  const out: Record<string, FieldValue> = {};
  for (const f of schema.fields) {
    if (f.default !== undefined) out[f.key] = f.default;
    else if (f.kind === "bool") out[f.key] = false;
    else if (f.kind === "select" && f.required) out[f.key] = f.options?.[0] ?? null;
    else out[f.key] = null;
  }
  return out;
}

/** Props of {@link SchemaForm}. */
export interface SchemaFormProps {
  /** Widget schema whose fields are rendered. */
  schema: WidgetSchema;
  /** Current field values keyed by field key. */
  values: Record<string, FieldValue>;
  /** Called with the full new value map on every change. */
  onChange(values: Record<string, FieldValue>): void;
  /** Error messages keyed by field key, shown under the inputs. */
  errors?: Record<string, string>;
}

/**
 * Renders one input per schema field: text/url inputs, number inputs,
 * checkboxes, selects, and password inputs for secrets.
 *
 * A secret that is already stored on the server arrives as
 * `{ $secret: "set" }`; it is shown masked with a **Replace** button (switches
 * to an empty password input) and a **Clear** button (sets `null`). While a
 * stored secret is being replaced, **Keep** puts the placeholder back.
 */
export function SchemaForm({ schema, values, onChange, errors = {} }: SchemaFormProps) {
  const baseId = useId();
  /** Keys of stored secrets the user chose to replace (so Keep can be offered). */
  const [replacing, setReplacing] = useState<ReadonlySet<string>>(() => new Set());

  const set = (key: string, value: FieldValue) => onChange({ ...values, [key]: value });
  const markReplacing = (key: string, on: boolean) =>
    setReplacing((prev) => {
      const next = new Set(prev);
      if (on) next.add(key);
      else next.delete(key);
      return next;
    });

  return (
    <div className="schema-form">
      {schema.fields.map((f) => {
        const id = `${baseId}-${f.key}`;
        const helpId = f.help ? `${id}-help` : undefined;
        const errId = errors[f.key] ? `${id}-err` : undefined;
        const describedBy = [helpId, errId].filter(Boolean).join(" ") || undefined;
        const common = {
          id,
          "aria-describedby": describedBy,
          "aria-invalid": errors[f.key] ? true : undefined,
          "aria-required": f.required || undefined,
        };
        const value = values[f.key];
        const label = (
          <label htmlFor={id} className={f.required ? "required" : undefined}>
            {f.label}
          </label>
        );
        const footer = (
          <>
            {f.help && (
              <span id={helpId} className="field-help">
                {f.help}
              </span>
            )}
            {errors[f.key] && (
              <span id={errId} className="field-error" role="alert">
                {errors[f.key]}
              </span>
            )}
          </>
        );

        if (f.kind === "bool") {
          return (
            <div key={f.key} className="field">
              <div className="field-inline">
                <input
                  {...common}
                  type="checkbox"
                  checked={value === true}
                  onChange={(e) => set(f.key, e.target.checked)}
                />
                {label}
              </div>
              {footer}
            </div>
          );
        }

        return (
          <div key={f.key} className="field">
            {label}
            <FieldInput
              field={f}
              value={value}
              common={common}
              replacing={replacing.has(f.key)}
              onValue={(v) => set(f.key, v)}
              onReplace={(on) => markReplacing(f.key, on)}
            />
            {footer}
          </div>
        );
      })}
    </div>
  );
}

/** Attributes every generated input shares. */
interface CommonAttrs {
  id: string;
  "aria-describedby"?: string;
  "aria-invalid"?: boolean;
  "aria-required"?: boolean;
}

/** The input for one non-boolean field. */
function FieldInput({
  field,
  value,
  common,
  replacing,
  onValue,
  onReplace,
}: {
  field: FieldSchema;
  value: FieldValue | undefined;
  common: CommonAttrs;
  replacing: boolean;
  onValue(v: FieldValue): void;
  onReplace(on: boolean): void;
}) {
  switch (field.kind) {
    case "number":
      return (
        <input
          {...common}
          type="number"
          value={typeof value === "number" || typeof value === "string" ? value : ""}
          onChange={(e) => {
            const raw = e.target.value;
            if (raw === "") onValue(null);
            else onValue(Number.isFinite(e.target.valueAsNumber) ? e.target.valueAsNumber : raw);
          }}
        />
      );
    case "select":
      return (
        <select
          {...common}
          value={value === null || value === undefined ? "" : String(value)}
          onChange={(e) => onValue(e.target.value === "" ? null : e.target.value)}
        >
          {!field.required && <option value="">(none)</option>}
          {field.required && (value === null || value === undefined) && (
            <option value="" disabled>
              Choose…
            </option>
          )}
          {(field.options ?? []).map((o) => (
            <option key={o} value={o}>
              {o}
            </option>
          ))}
        </select>
      );
    case "secret":
      if (isSecretPlaceholder(value)) {
        return (
          <div className="secret-row">
            <input {...common} type="password" value="" placeholder="••••••••" readOnly />
            <button
              type="button"
              className="btn"
              onClick={() => {
                onReplace(true);
                onValue("");
              }}
            >
              Replace
            </button>
            <button type="button" className="btn btn-danger" onClick={() => onValue(null)}>
              Clear
            </button>
          </div>
        );
      }
      return (
        <div className="secret-row">
          <input
            {...common}
            type="password"
            autoComplete="new-password"
            value={typeof value === "string" ? value : ""}
            onChange={(e) => onValue(e.target.value)}
          />
          {replacing && (
            <button
              type="button"
              className="btn"
              onClick={() => {
                onReplace(false);
                onValue({ ...SECRET_SET });
              }}
            >
              Keep
            </button>
          )}
        </div>
      );
    default:
      return (
        <input
          {...common}
          type={field.kind === "url" ? "url" : "text"}
          inputMode={field.kind === "url" ? "url" : undefined}
          spellCheck={false}
          value={typeof value === "string" || typeof value === "number" ? value : ""}
          onChange={(e) => onValue(e.target.value === "" ? null : e.target.value)}
        />
      );
  }
}
