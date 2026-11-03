/**
 * Drawer form for dashboard-wide settings: title, theme, accent colour,
 * background, search and custom CSS.
 */
import { useId } from "react";
import type { Background, SearchSettings, Settings, ThemeMode } from "../../api/types";
import { updateSettings } from "../ops";
import { useFormSession } from "./useFormSession";
import "./forms.css";

/** Props of {@link SettingsForm}. */
export interface SettingsFormProps {
  /** Closes the drawer. */
  onClose: () => void;
}

/** Background used when the admin turns the background image on. */
const NEW_BACKGROUND: Background = { url: "", blur: 0, opacity: 1 };

/** Accepts `#rgb` and `#rrggbb` hex colours, the values `<input type=color>` understands. */
const HEX = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i;

/** Expands `#abc` to `#aabbcc` for the colour input; other values fall back to a default. */
function toColorInput(value: string): string {
  if (!HEX.test(value)) return "#3b6fd8";
  if (value.length === 4) return `#${[...value.slice(1)].map((c) => c + c).join("")}`;
  return value.toLowerCase();
}

/** Clamps `n` into `[min, max]`, returning `fallback` for non-numbers. */
function clamp(n: number, min: number, max: number, fallback: number): number {
  return Number.isFinite(n) ? Math.min(max, Math.max(min, n)) : fallback;
}

/**
 * Settings form. Every change is applied with `updateSettings` so the
 * dashboard previews it live; Cancel reverts the whole session.
 */
export function SettingsForm({ onClose }: SettingsFormProps) {
  const session = useFormSession(onClose);
  const { settings } = session.doc;
  const uid = useId();
  const set = (patch: Partial<Settings>) => session.update(updateSettings(session.doc, patch));
  const setBg = (patch: Partial<Background>) =>
    set({ background: { ...(settings.background ?? NEW_BACKGROUND), ...patch } });
  const setSearch = (patch: Partial<SearchSettings>) =>
    set({ search: { ...settings.search, ...patch } });

  const bg = settings.background;
  const customUrl = settings.search.url ?? "";
  const searchError =
    settings.search.provider === "custom" && !customUrl.includes("%s")
      ? "The search URL must contain %s where the query goes"
      : null;
  const accentError = settings.accent.trim() === "" ? "Accent colour is required" : null;

  return (
    <form
      className="editor-form"
      onSubmit={(e) => {
        e.preventDefault();
        session.done();
      }}
    >
      <h2>Settings</h2>

      <div className="field">
        <label htmlFor={`${uid}-title`}>Title</label>
        <input
          id={`${uid}-title`}
          type="text"
          value={settings.title}
          onChange={(e) => set({ title: e.target.value })}
        />
      </div>

      <div className="field">
        <label htmlFor={`${uid}-theme`}>Theme</label>
        <select
          id={`${uid}-theme`}
          value={settings.theme}
          onChange={(e) => set({ theme: e.target.value as ThemeMode })}
        >
          <option value="system">Follow system</option>
          <option value="light">Light</option>
          <option value="dark">Dark</option>
        </select>
      </div>

      <div className="field">
        <label htmlFor={`${uid}-accent`}>Accent colour</label>
        <div className="field-inline">
          <input
            id={`${uid}-accent-picker`}
            type="color"
            aria-label="Accent colour picker"
            value={toColorInput(settings.accent)}
            onChange={(e) => set({ accent: e.target.value })}
          />
          <input
            id={`${uid}-accent`}
            type="text"
            spellCheck={false}
            value={settings.accent}
            aria-invalid={accentError ? true : undefined}
            onChange={(e) => set({ accent: e.target.value })}
          />
        </div>
        {accentError && (
          <span className="field-error" role="alert">
            {accentError}
          </span>
        )}
      </div>

      <fieldset className="form-section">
        <legend>Background</legend>
        <div className="field field-inline">
          <input
            id={`${uid}-nobg`}
            type="checkbox"
            checked={bg === null}
            onChange={(e) => set({ background: e.target.checked ? null : { ...NEW_BACKGROUND } })}
          />
          <label htmlFor={`${uid}-nobg`}>No background</label>
        </div>
        {bg && (
          <>
            <div className="field">
              <label htmlFor={`${uid}-bgurl`}>Image URL</label>
              <input
                id={`${uid}-bgurl`}
                type="url"
                spellCheck={false}
                placeholder="https://… or /icons/custom/…"
                value={bg.url}
                onChange={(e) => setBg({ url: e.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor={`${uid}-blur`}>Blur (px)</label>
              <div className="field-inline">
                <input
                  id={`${uid}-blur`}
                  type="range"
                  min={0}
                  max={30}
                  step={1}
                  value={bg.blur}
                  onChange={(e) => setBg({ blur: clamp(e.target.valueAsNumber, 0, 30, 0) })}
                />
                <output htmlFor={`${uid}-blur`}>{bg.blur}</output>
              </div>
            </div>
            <div className="field">
              <label htmlFor={`${uid}-opacity`}>Opacity</label>
              <div className="field-inline">
                <input
                  id={`${uid}-opacity`}
                  type="range"
                  min={0}
                  max={1}
                  step={0.05}
                  value={bg.opacity}
                  onChange={(e) => setBg({ opacity: clamp(e.target.valueAsNumber, 0, 1, 1) })}
                />
                <output htmlFor={`${uid}-opacity`}>{Math.round(bg.opacity * 100)}%</output>
              </div>
            </div>
          </>
        )}
      </fieldset>

      <fieldset className="form-section">
        <legend>Search</legend>
        <div className="field">
          <label htmlFor={`${uid}-provider`}>Search provider</label>
          <select
            id={`${uid}-provider`}
            value={settings.search.provider}
            onChange={(e) => {
              const provider = e.target.value as SearchSettings["provider"];
              setSearch(provider === "custom" ? { provider } : { provider, url: undefined });
            }}
          >
            <option value="duckduckgo">DuckDuckGo</option>
            <option value="google">Google</option>
            <option value="bing">Bing</option>
            <option value="custom">Custom</option>
          </select>
        </div>
        {settings.search.provider === "custom" && (
          <div className="field">
            <label htmlFor={`${uid}-searchurl`} className="required">
              Search URL
            </label>
            <input
              id={`${uid}-searchurl`}
              type="url"
              spellCheck={false}
              placeholder="https://search.example/?q=%s"
              value={customUrl}
              aria-required
              aria-invalid={searchError ? true : undefined}
              aria-describedby={`${uid}-searchurl-help`}
              onChange={(e) => setSearch({ url: e.target.value })}
            />
            <span id={`${uid}-searchurl-help`} className="field-help">
              %s is replaced by the search terms.
            </span>
            {searchError && (
              <span className="field-error" role="alert">
                {searchError}
              </span>
            )}
          </div>
        )}
      </fieldset>

      <div className="field">
        <label htmlFor={`${uid}-css`}>Custom CSS</label>
        <textarea
          id={`${uid}-css`}
          spellCheck={false}
          value={settings.customCss}
          aria-describedby={`${uid}-css-help`}
          onChange={(e) => set({ customCss: e.target.value })}
        />
        <span id={`${uid}-css-help`} className="field-help">
          Appended after the built-in styles.
        </span>
      </div>

      <div className="drawer-footer btn-row">
        <button
          type="submit"
          className="btn btn-primary"
          disabled={Boolean(searchError || accentError)}
        >
          Done
        </button>
        <button type="button" className="btn" onClick={session.cancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}
