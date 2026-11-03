/**
 * Renders a service or bookmark icon from a name or URL, with a text
 * fallback when there is no icon or it fails to load.
 */
import { useState } from "react";
import { iconUrl } from "../api/client";
import type { IconKind } from "../api/types";

/** Props of {@link Icon}. */
export interface IconProps {
  /** Icon name or URL. */
  icon?: string;
  /** `mask` draws a monochrome glyph tinted with the current text colour. */
  kind?: IconKind;
  /** Name used for the alt text and the fallback initials. */
  name: string;
  /** Size in pixels (square). */
  size?: number;
  /** Extra class names. */
  className?: string;
}

/** Initials for the fallback tile, e.g. "Home Assistant" → "HA". */
export function initials(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return "?";
  if (words.length === 1) return words[0]!.slice(0, 2).toUpperCase();
  return (words[0]![0]! + words[1]![0]!).toUpperCase();
}

/** An icon image, a tinted mask, or an initials tile. */
export function Icon({
  icon,
  kind = "image",
  name,
  size = 36,
  className = "card-icon",
}: IconProps) {
  const [failed, setFailed] = useState<string | null>(null);
  const url = iconUrl(icon);
  const style = { width: size, height: size, flexBasis: size };
  if (!url || failed === url) {
    return (
      <span
        className={`${className} fallback`}
        style={{ ...style, fontSize: size * 0.38 }}
        aria-hidden
      >
        {initials(name)}
      </span>
    );
  }
  if (kind === "mask") {
    return (
      <span
        className={className}
        role="img"
        aria-label={name}
        style={{
          ...style,
          backgroundColor: "currentColor",
          mask: `url("${url}") center / contain no-repeat`,
          WebkitMask: `url("${url}") center / contain no-repeat`,
        }}
      />
    );
  }
  return (
    <img
      className={className}
      src={url}
      alt=""
      width={size}
      height={size}
      loading="lazy"
      onError={() => setFailed(url)}
    />
  );
}
