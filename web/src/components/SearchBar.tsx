/**
 * Search box: filters cards as you type; Enter searches the web, or opens
 * the card directly when exactly one matches. Press "/" anywhere to focus.
 */
import { useEffect, useRef } from "react";
import type { SearchSettings } from "../api/types";

/** Props of {@link SearchBar}. */
export interface SearchBarProps {
  value: string;
  onChange(value: string): void;
  settings: SearchSettings;
  /** Href of the single matching card, if exactly one matches. */
  singleMatch?: string;
}

/** Builds the web search URL for a query. Exported for tests. */
export function searchUrl(settings: SearchSettings, q: string): string {
  const enc = encodeURIComponent(q);
  switch (settings.provider) {
    case "google":
      return `https://www.google.com/search?q=${enc}`;
    case "bing":
      return `https://www.bing.com/search?q=${enc}`;
    case "custom":
      if (settings.url?.includes("%s")) return settings.url.replace("%s", enc);
      return `https://duckduckgo.com/?q=${enc}`;
    case "duckduckgo":
    default:
      return `https://duckduckgo.com/?q=${enc}`;
  }
}

/** The search input. */
export function SearchBar({ value, onChange, settings, singleMatch }: SearchBarProps) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (e.key !== "/" || t?.closest("input, textarea, select, [contenteditable]")) return;
      e.preventDefault();
      ref.current?.focus();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  return (
    <form
      className="search"
      role="search"
      onSubmit={(e) => {
        e.preventDefault();
        const q = value.trim();
        if (!q) return;
        window.open(singleMatch ?? searchUrl(settings, q), "_blank", "noopener,noreferrer");
      }}
    >
      <svg
        width="16"
        height="16"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        aria-hidden
      >
        <circle cx="11" cy="11" r="7" />
        <path d="m20 20-3.5-3.5" />
      </svg>
      <input
        ref={ref}
        type="search"
        aria-label="Search services or the web"
        placeholder="Search services or the web  ( / )"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") onChange("");
        }}
      />
    </form>
  );
}

/** Whether a card matches the filter text (name, description, URL). */
export function matches(q: string, ...fields: (string | undefined)[]): boolean {
  const needle = q.trim().toLowerCase();
  if (!needle) return true;
  return fields.some((f) => f?.toLowerCase().includes(needle));
}
