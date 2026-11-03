/**
 * The strip of info widgets above the groups: clock, weather, host
 * resources and Docker container counts.
 *
 * The clock runs entirely in the browser. The other widgets poll
 * `GET /api/info/{widgetId}` through {@link Api.infoData}; the server makes
 * the upstream requests.
 */
import { useEffect, useState } from "react";
import type { Api } from "../../api/client";
import type { DockerData, InfoWidget, ResourcesData, WeatherData } from "../../api/types";
import { useApp } from "../../app/context";
import { formatBytes } from "./format";
import { usePolling } from "./usePolling";
import "./widgets.css";

export { formatBytes } from "./format";

/** How often the resources widget refreshes, in milliseconds. */
export const RESOURCES_INTERVAL_MS = 10_000;
/** How often the Docker widget refreshes, in milliseconds. */
export const DOCKER_INTERVAL_MS = 30_000;
/** How often the weather widget refreshes, in milliseconds. */
export const WEATHER_INTERVAL_MS = 10 * 60_000;

/** Props of {@link InfoWidgetStrip}. */
export interface InfoWidgetStripProps {
  /** Widgets to render, in display order. */
  widgets: InfoWidget[];
}

/**
 * Renders the info widgets in order. Unknown widget types are skipped.
 * Renders nothing when the list is empty.
 */
export function InfoWidgetStrip({ widgets }: InfoWidgetStripProps) {
  if (widgets.length === 0) return null;
  return (
    <div className="info-strip">
      {widgets.map((w) => {
        switch (w.type) {
          case "clock":
            return <ClockWidget key={w.id} widget={w} />;
          case "weather":
            return <WeatherWidget key={w.id} widget={w} />;
          case "resources":
            return <ResourcesWidget key={w.id} widget={w} />;
          case "docker":
            return <DockerWidget key={w.id} widget={w} />;
          default:
            return null;
        }
      })}
    </div>
  );
}

/** Props shared by the individual info widgets. */
interface WidgetProps {
  widget: InfoWidget;
}

const isNum = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v);
const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null;

/** Fetches an info widget payload and checks its shape. */
async function fetchInfo<T>(api: Api, id: string, check: (v: unknown) => v is T): Promise<T> {
  const raw = await api.infoData(id);
  if (!check(raw)) throw new Error("Unexpected response");
  return raw;
}

const isWeather = (v: unknown): v is WeatherData =>
  isObj(v) && isNum(v.tempC) && typeof v.condition === "string";
const isResources = (v: unknown): v is ResourcesData =>
  isObj(v) &&
  isNum(v.cpu) &&
  isNum(v.memUsed) &&
  isNum(v.memTotal) &&
  isNum(v.diskUsed) &&
  isNum(v.diskTotal);
const isDocker = (v: unknown): v is DockerData => isObj(v) && isNum(v.running) && isNum(v.stopped);

/** Quiet placeholder while a widget's first payload loads. */
function Loading({ label }: { label: string }) {
  return (
    <span className="info-loading" aria-busy="true" aria-label={`${label} loading`}>
      {label} …
    </span>
  );
}

/** Compact error state; the full message is in the tooltip. */
function Failed({ label, error }: { label: string; error: string }) {
  return (
    <span className="info-error" role="status" title={error}>
      {label} unavailable
    </span>
  );
}

/** Local time (12 or 24 hour, per `options.format`) with the date below. */
function ClockWidget({ widget }: WidgetProps) {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const timer = setInterval(() => setNow(new Date()), 1000);
    return () => clearInterval(timer);
  }, []);
  const hour12 = widget.options.format === "12h";
  const time = now.toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12,
  });
  const date = now.toLocaleDateString(undefined, {
    weekday: "short",
    day: "numeric",
    month: "short",
  });
  return (
    <div className="info-widget info-clock">
      <time className="info-clock-time" dateTime={now.toISOString()}>
        {time}
      </time>
      <span className="info-sub">{date}</span>
    </div>
  );
}

/** Current temperature and condition; `options.units: "imperial"` shows °F. */
function WeatherWidget({ widget }: WidgetProps) {
  const { api } = useApp();
  const { data, error, loading } = usePolling(
    () => fetchInfo(api, widget.id, isWeather),
    WEATHER_INTERVAL_MS,
    [widget.id],
  );
  if (loading) return <Loading label="Weather" />;
  if (!data) return <Failed label="Weather" error={error ?? "No data"} />;
  const imperial = widget.options.units === "imperial";
  const temp = imperial ? (data.tempC * 9) / 5 + 32 : data.tempC;
  const location = typeof widget.options.location === "string" ? widget.options.location : "";
  return (
    <div className="info-widget info-weather">
      <span className="info-main">
        {Math.round(temp)}°{imperial ? "F" : "C"}
      </span>
      <span className="info-sub">
        {data.condition}
        {location && ` · ${location}`}
      </span>
    </div>
  );
}

/** One labelled usage bar. */
function Meter({ label, ratio, text }: { label: string; ratio: number; text: string }) {
  const pct = Math.max(0, Math.min(100, ratio * 100));
  return (
    <div className="info-meter-row">
      <span className="info-meter-label">{label}</span>
      <span
        className={`info-meter${pct >= 90 ? " is-high" : ""}`}
        role="meter"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(pct)}
        aria-valuetext={text}
      >
        <span className="info-meter-fill" style={{ width: `${pct}%` }} />
      </span>
      <span className="info-meter-text">{text}</span>
    </div>
  );
}

/** Host CPU, memory and disk usage. */
function ResourcesWidget({ widget }: WidgetProps) {
  const { api } = useApp();
  const { data, error, loading } = usePolling(
    () => fetchInfo(api, widget.id, isResources),
    RESOURCES_INTERVAL_MS,
    [widget.id],
  );
  if (loading) return <Loading label="Resources" />;
  if (!data) return <Failed label="Resources" error={error ?? "No data"} />;
  const ratio = (used: number, total: number) => (total > 0 ? used / total : 0);
  return (
    <div className="info-widget info-resources">
      <Meter label="CPU" ratio={data.cpu / 100} text={`${Math.round(data.cpu)}%`} />
      <Meter
        label="RAM"
        ratio={ratio(data.memUsed, data.memTotal)}
        text={`${formatBytes(data.memUsed)} / ${formatBytes(data.memTotal)}`}
      />
      <Meter
        label="Disk"
        ratio={ratio(data.diskUsed, data.diskTotal)}
        text={`${formatBytes(data.diskUsed)} / ${formatBytes(data.diskTotal)}`}
      />
    </div>
  );
}

/** Running and stopped container counts. */
function DockerWidget({ widget }: WidgetProps) {
  const { api } = useApp();
  const { data, error, loading } = usePolling(
    () => fetchInfo(api, widget.id, isDocker),
    DOCKER_INTERVAL_MS,
    [widget.id],
  );
  if (loading) return <Loading label="Docker" />;
  if (!data) return <Failed label="Docker" error={error ?? "No data"} />;
  return (
    <div className="info-widget info-docker">
      <span className="info-main">
        <span className="info-count is-up">{data.running}</span> running
      </span>
      <span className="info-sub">
        <span className={data.stopped > 0 ? "info-count is-down" : "info-count"}>
          {data.stopped}
        </span>{" "}
        stopped
      </span>
    </div>
  );
}
