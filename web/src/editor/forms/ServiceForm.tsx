/**
 * Drawer form for adding or editing a service card: details, icon, group,
 * status check and service widget (schema-generated fields with a
 * "Test connection" button).
 */
import { useId, useState } from "react";
import { useApp } from "../../app/context";
import type {
  Check,
  CheckKind,
  FieldValue,
  Service,
  TestResult,
  WidgetSchema,
} from "../../api/types";
import { deleteService, findService, findServiceGroup, newId, upsertService } from "../ops";
import { IconPicker } from "./IconPicker";
import { SchemaForm, defaultValues, validateWidget } from "./SchemaForm";
import { useFormSession } from "./useFormSession";
import { groupOptions, isHttpUrl, optional } from "./util";
import "./forms.css";

/** Props of {@link ServiceForm}. */
export interface ServiceFormProps {
  /** Group the card belongs to (or is added to, for a new card). */
  groupId: string;
  /** Card being edited; omitted to add a new card. */
  serviceId?: string;
  /** Closes the drawer. */
  onClose: () => void;
}

/**
 * Validates the parts of a service the form edits, widget fields included.
 *
 * @param catalog widget catalogue used to look up the widget schema.
 * @returns error messages keyed by `name`, `href`, `expectStatus` or
 *   `widget.<fieldKey>`.
 */
export function validateService(svc: Service, catalog: WidgetSchema[]): Record<string, string> {
  const errors: Record<string, string> = {};
  if (svc.name.trim() === "") errors.name = "Name is required";
  if (svc.href && !isHttpUrl(svc.href) && !svc.href.startsWith("/")) {
    errors.href = "Link must start with http://, https:// or /";
  }
  const code = svc.check?.expectStatus;
  if (code !== undefined && !(Number.isInteger(code) && code >= 100 && code <= 599)) {
    errors.expectStatus = "Expected status must be a number from 100 to 599";
  }
  if (svc.widget) {
    const schema = catalog.find((w) => w.type === svc.widget!.type);
    if (schema) {
      for (const [k, v] of Object.entries(validateWidget(schema, svc.widget.fields))) {
        errors[`widget.${k}`] = v;
      }
    }
  }
  return errors;
}

/** Groups catalogue entries by category, categories and names sorted. */
function byCategory(catalog: WidgetSchema[]): [string, WidgetSchema[]][] {
  const map = new Map<string, WidgetSchema[]>();
  for (const w of catalog) map.set(w.category, [...(map.get(w.category) ?? []), w]);
  return [...map.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([c, list]) => [c, [...list].sort((a, b) => a.name.localeCompare(b.name))]);
}

/**
 * Add/edit form for a service card. Every change is applied to the working
 * copy right away (live preview); Cancel reverts the whole session.
 */
export function ServiceForm({ groupId, serviceId, onClose }: ServiceFormProps) {
  const { api, catalog } = useApp();
  const session = useFormSession(onClose);
  const { doc } = session;
  const uid = useId();
  const [id] = useState(() => serviceId ?? newId("s"));
  const [touched, setTouched] = useState(Boolean(serviceId));
  const [test, setTest] = useState<TestResult | null>(null);
  const [testing, setTesting] = useState(false);

  const existing = findService(doc, id);
  const svc: Service = existing ?? { id, name: "", check: null, widget: null };
  const currentGroup = findServiceGroup(doc, id)?.id ?? groupId;
  const schema = svc.widget ? catalog.find((w) => w.type === svc.widget!.type) : undefined;
  const errors = validateService(svc, catalog);
  const valid = Object.keys(errors).length === 0;
  const shown = touched ? errors : {};
  const widgetErrors = Object.fromEntries(
    Object.entries(shown)
      .filter(([k]) => k.startsWith("widget."))
      .map(([k, v]) => [k.slice(7), v]),
  );

  const change = (patch: Partial<Service>, toGroup = currentGroup) => {
    setTouched(true);
    session.update(upsertService(doc, toGroup, { ...svc, ...patch }));
  };
  const changeCheck = (patch: Partial<Check> | null) => {
    if (patch === null) change({ check: null });
    else change({ check: { kind: "http", ...svc.check, ...patch } });
  };

  const chooseWidget = (type: string) => {
    setTest(null);
    if (type === "") return change({ widget: null });
    const ws = catalog.find((w) => w.type === type);
    const fields = ws ? defaultValues(ws) : {};
    if (ws?.fields.some((f) => f.key === "url") && fields.url == null && svc.href) {
      fields.url = svc.href;
    }
    change({ widget: { type, fields } });
  };

  const runTest = async () => {
    if (!svc.widget) return;
    setTesting(true);
    setTest(null);
    try {
      setTest(
        await api.testWidget({ type: svc.widget.type, fields: svc.widget.fields }, serviceId),
      );
    } catch (e) {
      setTest({ ok: false, message: e instanceof Error ? e.message : String(e) });
    } finally {
      setTesting(false);
    }
  };

  const remove = () => {
    if (!window.confirm(`Delete "${svc.name || "this service"}"?`)) return;
    session.update(deleteService(doc, id));
    session.done();
  };

  const err = (key: string) =>
    shown[key] ? (
      <span id={`${uid}-${key}-err`} className="field-error" role="alert">
        {shown[key]}
      </span>
    ) : null;
  const described = (key: string, help?: boolean) =>
    [help ? `${uid}-${key}-help` : "", shown[key] ? `${uid}-${key}-err` : ""]
      .filter(Boolean)
      .join(" ") || undefined;

  const checkKind = svc.check?.kind ?? "";

  return (
    <form
      className="editor-form"
      onSubmit={(e) => {
        e.preventDefault();
        if (valid) session.done();
      }}
    >
      <h2>{serviceId ? "Edit service" : "Add service"}</h2>

      <div className="field">
        <label htmlFor={`${uid}-name`} className="required">
          Name
        </label>
        <input
          id={`${uid}-name`}
          type="text"
          value={svc.name}
          aria-required
          aria-invalid={shown.name ? true : undefined}
          aria-describedby={described("name")}
          onChange={(e) => change({ name: e.target.value })}
        />
        {err("name")}
      </div>

      <div className="field">
        <label htmlFor={`${uid}-href`}>Link</label>
        <input
          id={`${uid}-href`}
          type="url"
          inputMode="url"
          spellCheck={false}
          placeholder="http://sonarr.lan:8989"
          value={svc.href ?? ""}
          aria-invalid={shown.href ? true : undefined}
          aria-describedby={described("href")}
          onChange={(e) => change({ href: optional(e.target.value) })}
        />
        {err("href")}
      </div>

      <div className="field">
        <label htmlFor={`${uid}-desc`}>Description</label>
        <input
          id={`${uid}-desc`}
          type="text"
          value={svc.description ?? ""}
          onChange={(e) => change({ description: optional(e.target.value) })}
        />
      </div>

      <IconPicker
        value={svc.icon}
        name={svc.name}
        onChange={(icon) => change({ icon, iconKind: undefined })}
      />

      <div className="field">
        <label htmlFor={`${uid}-group`}>Group</label>
        <select
          id={`${uid}-group`}
          value={currentGroup}
          onChange={(e) => change({}, e.target.value)}
        >
          {groupOptions(doc.groups).map((g) => (
            <option key={g.id} value={g.id}>
              {g.label}
            </option>
          ))}
        </select>
      </div>

      <fieldset className="form-section">
        <legend>Status check</legend>
        <div className="field">
          <label htmlFor={`${uid}-check`}>Check</label>
          <select
            id={`${uid}-check`}
            value={checkKind}
            onChange={(e) =>
              e.target.value === ""
                ? changeCheck(null)
                : changeCheck({
                    kind: e.target.value as CheckKind,
                    expectStatus: e.target.value === "http" ? svc.check?.expectStatus : undefined,
                  })
            }
          >
            <option value="">None</option>
            <option value="http">HTTP</option>
            <option value="tcp">TCP connect</option>
            <option value="ping">Ping</option>
          </select>
        </div>
        {svc.check && (
          <div className="field">
            <label htmlFor={`${uid}-target`}>Target</label>
            <input
              id={`${uid}-target`}
              type="text"
              spellCheck={false}
              value={svc.check.target ?? ""}
              placeholder={
                svc.check.kind === "http" ? (svc.href ?? "http://host:port/health") : "host:port"
              }
              aria-describedby={`${uid}-target-help`}
              onChange={(e) => changeCheck({ target: optional(e.target.value) })}
            />
            <span id={`${uid}-target-help`} className="field-help">
              {svc.check.kind === "http"
                ? "URL to request; the link is used when empty."
                : "Host (and port for TCP); the link's host is used when empty."}
            </span>
          </div>
        )}
        {svc.check?.kind === "http" && (
          <div className="field">
            <label htmlFor={`${uid}-expectStatus`}>Expected status</label>
            <input
              id={`${uid}-expectStatus`}
              type="number"
              min={100}
              max={599}
              placeholder="any 2xx/3xx"
              value={svc.check.expectStatus ?? ""}
              aria-invalid={shown.expectStatus ? true : undefined}
              aria-describedby={described("expectStatus")}
              onChange={(e) =>
                changeCheck({
                  expectStatus: e.target.value === "" ? undefined : e.target.valueAsNumber,
                })
              }
            />
            {err("expectStatus")}
          </div>
        )}
      </fieldset>

      <fieldset className="form-section">
        <legend>Widget</legend>
        <div className="field">
          <label htmlFor={`${uid}-widget`}>Widget type</label>
          <select
            id={`${uid}-widget`}
            value={svc.widget?.type ?? ""}
            onChange={(e) => chooseWidget(e.target.value)}
          >
            <option value="">None</option>
            {svc.widget && !schema && <option value={svc.widget.type}>{svc.widget.type}</option>}
            {byCategory(catalog).map(([cat, list]) => (
              <optgroup key={cat} label={cat}>
                {list.map((w) => (
                  <option key={w.type} value={w.type}>
                    {w.name}
                  </option>
                ))}
              </optgroup>
            ))}
          </select>
          {svc.widget && !schema && (
            <span className="field-help">
              This widget type is not in the catalogue; its settings are kept as they are.
            </span>
          )}
        </div>
        {svc.widget && schema && (
          <>
            <SchemaForm
              key={svc.widget.type}
              schema={schema}
              values={svc.widget.fields}
              errors={widgetErrors}
              onChange={(fields: Record<string, FieldValue>) => {
                setTest(null);
                change({ widget: { type: svc.widget!.type, fields } });
              }}
            />
            <div className="btn-row">
              <button
                type="button"
                className="btn"
                disabled={testing}
                onClick={() => void runTest()}
              >
                {testing ? "Testing…" : "Test connection"}
              </button>
            </div>
            {test && (
              <p className={`test-result ${test.ok ? "ok" : "fail"}`} role="status">
                {test.ok ? "✓ " : "✗ "}
                {test.message}
              </p>
            )}
          </>
        )}
      </fieldset>

      <div className="drawer-footer btn-row">
        <button type="submit" className="btn btn-primary" disabled={!valid}>
          Done
        </button>
        <button type="button" className="btn" onClick={session.cancel}>
          Cancel
        </button>
        {serviceId && existing && (
          <button type="button" className="btn btn-danger" onClick={remove}>
            Delete
          </button>
        )}
      </div>
    </form>
  );
}
