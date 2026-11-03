/**
 * Tiny inline chart of recent response times, with down samples marked.
 */
import type { StatusSample } from "../api/types";

/** Props of {@link Sparkline}. */
export interface SparklineProps {
  /** Samples, oldest first. */
  samples: StatusSample[];
  /** Width in pixels. */
  width?: number;
  /** Height in pixels. */
  height?: number;
}

/**
 * Builds the SVG path of the latency line. Down samples and samples without a
 * time break the line. Exported for tests.
 */
export function sparkPath(samples: StatusSample[], width: number, height: number): string {
  const values = samples.map((s) => (s.up && s.ms !== undefined ? s.ms : null));
  const max = Math.max(1, ...values.filter((v): v is number => v !== null));
  const step = samples.length > 1 ? width / (samples.length - 1) : 0;
  let d = "";
  let pen = false;
  values.forEach((v, i) => {
    if (v === null) {
      pen = false;
      return;
    }
    const x = (i * step).toFixed(1);
    const y = (height - 1 - (v / max) * (height - 2)).toFixed(1);
    d += `${pen ? "L" : "M"}${x} ${y}`;
    pen = true;
  });
  return d;
}

/** An SVG sparkline; renders nothing with fewer than two samples. */
export function Sparkline({ samples, width = 96, height = 22 }: SparklineProps) {
  if (samples.length < 2) return null;
  const step = width / (samples.length - 1);
  const upCount = samples.filter((s) => s.up).length;
  const pct = Math.round((upCount / samples.length) * 100);
  return (
    <svg
      className="sparkline"
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label={`Up ${pct}% of the last ${samples.length} checks`}
    >
      <title>{`Up ${pct}% of the last ${samples.length} checks`}</title>
      {samples.map((s, i) =>
        s.up ? null : (
          <rect
            key={i}
            x={i * step - 1}
            y={0}
            width={2}
            height={height}
            fill="var(--down)"
            opacity={0.7}
          />
        ),
      )}
      <path
        d={sparkPath(samples, width, height)}
        fill="none"
        stroke="currentColor"
        strokeWidth={1.5}
      />
    </svg>
  );
}
