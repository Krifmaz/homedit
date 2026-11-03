/**
 * Formatting helpers for widget values.
 */

const UNITS = ["B", "kB", "MB", "GB", "TB", "PB", "EB"] as const;

/**
 * Formats a byte count for humans with decimal (SI) units, the way disk and
 * memory sizes are usually labelled: `1500` → `"1.5 kB"`, `16e9` → `"16 GB"`.
 *
 * Values are rounded to `digits` decimals and trailing zeros are dropped.
 * Negative, `NaN` or infinite input gives `"–"`.
 *
 * @param bytes size in bytes.
 * @param digits maximum number of decimals (1 by default).
 */
export function formatBytes(bytes: number, digits = 1): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "–";
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1000;
    unit++;
  }
  // Rounding can push a value to 1000 (e.g. 999.96 kB); move up one unit then.
  if (Number(value.toFixed(digits)) >= 1000 && unit < UNITS.length - 1) {
    value /= 1000;
    unit++;
  }
  const shown = unit === 0 ? Math.round(value) : Number(value.toFixed(digits));
  return `${shown} ${UNITS[unit]}`;
}

/**
 * Formats a widget stat value: numbers get locale digit grouping, strings
 * are shown as they are.
 *
 * @param value the stat value from the API.
 */
export function formatStatValue(value: string | number): string {
  if (typeof value === "number") {
    return Number.isFinite(value)
      ? value.toLocaleString(undefined, { maximumFractionDigits: 2 })
      : "–";
  }
  return value;
}
