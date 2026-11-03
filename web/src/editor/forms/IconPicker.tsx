/**
 * Icon chooser used by the service and bookmark forms: a free-text icon
 * reference with live preview, a debounced search over built-in and
 * uploaded icons, and a custom upload.
 */
import { useEffect, useId, useRef, useState } from "react";
import { useApp } from "../../app/context";
import type { IconResult } from "../../api/types";
import { Icon } from "../../components/Icon";
import "./forms.css";

/** Delay between the last keystroke in the search box and the search request. */
export const ICON_SEARCH_DEBOUNCE_MS = 250;

/** Props of {@link IconPicker}. */
export interface IconPickerProps {
  /** Current icon reference (name such as `sonarr.png`, `sh-plex`, or a URL). */
  value: string | undefined;
  /** Called with the new reference, or `undefined` when cleared. */
  onChange(value: string | undefined): void;
  /** Name of the thing the icon is for; used for the preview fallback. */
  name: string;
  /** Label of the text input; `Icon` by default. */
  label?: string;
}

/**
 * Text input for an icon reference with preview, search results grid and
 * an upload button.
 */
export function IconPicker({ value, onChange, name, label = "Icon" }: IconPickerProps) {
  const { api } = useApp();
  const id = useId();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<IconResult[]>([]);
  const [searching, setSearching] = useState(false);
  /** Query the current `results` answer, to tell "no results" from "not searched yet". */
  const [answered, setAnswered] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [uploading, setUploading] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const q = query.trim();
    let alive = true;
    const timer = setTimeout(
      () => {
        if (!q) {
          setResults([]);
          setSearching(false);
          return;
        }
        setSearching(true);
        api.searchIcons(q).then(
          (r) => {
            if (!alive) return;
            setResults(r);
            setAnswered(q);
            setError(null);
            setSearching(false);
          },
          (e: unknown) => {
            if (!alive) return;
            setResults([]);
            setError(e instanceof Error ? e.message : String(e));
            setSearching(false);
          },
        );
      },
      q ? ICON_SEARCH_DEBOUNCE_MS : 0,
    );
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [api, query]);

  const upload = async (file: File) => {
    setUploading(true);
    try {
      const icon = await api.uploadIcon(file);
      onChange(icon.name);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setUploading(false);
      if (fileRef.current) fileRef.current.value = "";
    }
  };

  return (
    <div className="field icon-picker">
      <label htmlFor={`${id}-ref`}>{label}</label>
      <div className="icon-picker-row">
        <span className="icon-picker-preview" data-testid="icon-preview">
          <Icon icon={value} name={name || "?"} size={32} className="card-icon" />
        </span>
        <input
          id={`${id}-ref`}
          type="text"
          spellCheck={false}
          value={value ?? ""}
          placeholder="sonarr.png, sh-plex, mdi-server or a URL"
          aria-describedby={`${id}-help`}
          onChange={(e) => onChange(e.target.value === "" ? undefined : e.target.value)}
        />
      </div>
      <span id={`${id}-help`} className="field-help">
        A dashboard-icons name, a selfh.st name (sh-…), an uploaded icon, or a URL.
      </span>
      <label htmlFor={`${id}-search`} className="sr-only">
        Search icons
      </label>
      <div className="icon-picker-row">
        <input
          id={`${id}-search`}
          type="search"
          value={query}
          placeholder="Search icons…"
          onChange={(e) => setQuery(e.target.value)}
        />
        <input
          ref={fileRef}
          id={`${id}-file`}
          type="file"
          accept="image/*"
          className="sr-only"
          aria-label="Upload icon file"
          onChange={(e) => {
            const file = e.target.files?.[0];
            if (file) void upload(file);
          }}
        />
        <button
          type="button"
          className="btn"
          disabled={uploading}
          onClick={() => fileRef.current?.click()}
        >
          {uploading ? "Uploading…" : "Upload"}
        </button>
      </div>
      {error && (
        <span className="field-error" role="alert">
          {error}
        </span>
      )}
      {searching && <span className="field-help">Searching…</span>}
      {results.length > 0 && (
        <div className="icon-grid" role="group" aria-label="Icon search results">
          {results.map((r) => (
            <button
              key={`${r.source}:${r.name}`}
              type="button"
              className={`btn icon-choice${r.name === value ? " selected" : ""}`}
              title={`${r.name} (${r.source})`}
              aria-label={`Use icon ${r.name}`}
              aria-pressed={r.name === value}
              onClick={() => onChange(r.name)}
            >
              <img src={r.url} alt="" width={28} height={28} loading="lazy" />
            </button>
          ))}
        </div>
      )}
      {!searching && query.trim() !== "" && answered === query.trim() && results.length === 0 && (
        <span className="field-help">No icons found.</span>
      )}
    </div>
  );
}
