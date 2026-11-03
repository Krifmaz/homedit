/**
 * Drawer form for the info widgets in the header strip: add, remove,
 * reorder, and per-type options.
 */
import { useId, useState } from "react";
import type { InfoWidget, InfoWidgetType } from "../../api/types";
import { newId, produce } from "../ops";
import { useFormSession } from "./useFormSession";
import "./forms.css";

/** Props of {@link InfoWidgetsForm}. */
export interface InfoWidgetsFormProps {
  /** Closes the drawer. */
  onClose: () => void;
}

/** Info widget types with their display names, in picker order. */
export const INFO_WIDGET_TYPES: { type: InfoWidgetType; label: string }[] = [
  { type: "clock", label: "Clock" },
  { type: "weather", label: "Weather" },
  { type: "resources", label: "Host resources" },
  { type: "docker", label: "Docker" },
];

/** Default options for a newly added info widget of `type`. */
export function defaultInfoOptions(type: InfoWidgetType): InfoWidget["options"] {
  switch (type) {
    case "clock":
      return { format: "24h" };
    case "weather":
      return { units: "metric" };
    default:
      return {};
  }
}

/** Display name of an info widget type. */
function labelOf(type: InfoWidgetType): string {
  return INFO_WIDGET_TYPES.find((t) => t.type === type)?.label ?? type;
}

/** One option value of an info widget. */
type OptionValue = string | number | boolean;

/**
 * Lists the info widgets with add, remove, move up/down and per-type
 * options. Edits preview live; Cancel reverts the session.
 */
export function InfoWidgetsForm({ onClose }: InfoWidgetsFormProps) {
  const session = useFormSession(onClose);
  const { doc } = session;
  const uid = useId();
  const [addType, setAddType] = useState<InfoWidgetType>("clock");
  const widgets = doc.infoWidgets;

  const edit = (recipe: (list: InfoWidget[]) => void) =>
    session.update(produce(doc, (d) => recipe(d.infoWidgets)));

  const add = () =>
    edit((list) => {
      list.push({ id: newId("iw"), type: addType, options: defaultInfoOptions(addType) });
    });
  const remove = (id: string) =>
    edit((list) => {
      const i = list.findIndex((w) => w.id === id);
      if (i >= 0) list.splice(i, 1);
    });
  const move = (id: string, delta: number) =>
    edit((list) => {
      const i = list.findIndex((w) => w.id === id);
      const j = i + delta;
      if (i < 0 || j < 0 || j >= list.length) return;
      [list[i], list[j]] = [list[j]!, list[i]!];
    });
  const setOption = (id: string, key: string, value: OptionValue | undefined) =>
    edit((list) => {
      const w = list.find((x) => x.id === id);
      if (!w) return;
      if (value === undefined || value === "") delete w.options[key];
      else w.options[key] = value;
    });

  return (
    <form
      className="editor-form"
      onSubmit={(e) => {
        e.preventDefault();
        session.done();
      }}
    >
      <h2>Info widgets</h2>

      {widgets.length === 0 && <p className="field-help">No info widgets yet.</p>}
      <ol className="info-widget-list">
        {widgets.map((w, i) => {
          const name = `${labelOf(w.type)} ${widgets.filter((x, j) => x.type === w.type && j <= i).length}`;
          return (
            <li key={w.id} className="info-widget-item">
              <fieldset className="form-section">
                <legend>{labelOf(w.type)}</legend>
                <InfoOptions
                  widget={w}
                  idPrefix={`${uid}-${w.id}`}
                  onOption={(k, v) => setOption(w.id, k, v)}
                />
                <div className="btn-row">
                  <button
                    type="button"
                    className="btn btn-icon"
                    aria-label={`Move ${name} up`}
                    disabled={i === 0}
                    onClick={() => move(w.id, -1)}
                  >
                    ↑
                  </button>
                  <button
                    type="button"
                    className="btn btn-icon"
                    aria-label={`Move ${name} down`}
                    disabled={i === widgets.length - 1}
                    onClick={() => move(w.id, 1)}
                  >
                    ↓
                  </button>
                  <button
                    type="button"
                    className="btn btn-danger"
                    aria-label={`Remove ${name}`}
                    onClick={() => remove(w.id)}
                  >
                    Remove
                  </button>
                </div>
              </fieldset>
            </li>
          );
        })}
      </ol>

      <div className="field">
        <label htmlFor={`${uid}-add`}>Add widget</label>
        <div className="field-inline">
          <select
            id={`${uid}-add`}
            value={addType}
            onChange={(e) => setAddType(e.target.value as InfoWidgetType)}
          >
            {INFO_WIDGET_TYPES.map((t) => (
              <option key={t.type} value={t.type}>
                {t.label}
              </option>
            ))}
          </select>
          <button type="button" className="btn" onClick={add}>
            Add
          </button>
        </div>
      </div>

      <div className="drawer-footer btn-row">
        <button type="submit" className="btn btn-primary">
          Done
        </button>
        <button type="button" className="btn" onClick={session.cancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}

/** Per-type option inputs of one info widget. */
function InfoOptions({
  widget,
  idPrefix,
  onOption,
}: {
  widget: InfoWidget;
  idPrefix: string;
  onOption(key: string, value: OptionValue | undefined): void;
}) {
  const o = widget.options;
  const str = (k: string) => (o[k] === undefined ? "" : String(o[k]));
  const num = (raw: string) => {
    if (raw.trim() === "") return undefined;
    const n = Number(raw);
    return Number.isFinite(n) ? n : raw;
  };
  const text = (key: string, label: string, placeholder?: string, numeric = false) => (
    <div className="field">
      <label htmlFor={`${idPrefix}-${key}`}>{label}</label>
      <input
        id={`${idPrefix}-${key}`}
        type={numeric ? "number" : "text"}
        step={numeric ? "any" : undefined}
        placeholder={placeholder}
        value={str(key)}
        onChange={(e) => onOption(key, numeric ? num(e.target.value) : e.target.value)}
      />
    </div>
  );
  const select = (key: string, label: string, choices: [string, string][], fallback: string) => (
    <div className="field">
      <label htmlFor={`${idPrefix}-${key}`}>{label}</label>
      <select
        id={`${idPrefix}-${key}`}
        value={str(key) || fallback}
        onChange={(e) => onOption(key, e.target.value)}
      >
        {choices.map(([v, l]) => (
          <option key={v} value={v}>
            {l}
          </option>
        ))}
      </select>
    </div>
  );

  switch (widget.type) {
    case "clock":
      return select(
        "format",
        "Time format",
        [
          ["24h", "24-hour"],
          ["12h", "12-hour"],
        ],
        "24h",
      );
    case "weather":
      return (
        <>
          {text("location", "Location name", "Home")}
          {text("latitude", "Latitude", "52.37", true)}
          {text("longitude", "Longitude", "4.90", true)}
          {select(
            "units",
            "Units",
            [
              ["metric", "Metric (°C)"],
              ["imperial", "Imperial (°F)"],
            ],
            "metric",
          )}
        </>
      );
    case "resources":
      return text("disk", "Disk path", "/");
    case "docker":
      return text("host", "Docker host", "local socket");
    default:
      return null;
  }
}
