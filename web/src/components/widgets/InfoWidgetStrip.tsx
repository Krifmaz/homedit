import type { InfoWidget } from "../../api/types";
/** Placeholder; replaced by the real strip. */
export function InfoWidgetStrip({ widgets }: { widgets: InfoWidget[] }) {
  return <div className="info-strip">{widgets.length} widgets</div>;
}
