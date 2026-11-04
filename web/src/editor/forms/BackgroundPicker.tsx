/**
 * Background image chooser for the Appearance form: a URL field with a
 * live preview, an upload button, and a gallery of images already uploaded
 * to the server (`/images/`).
 */
import { useEffect, useId, useRef, useState } from "react";
import { useApp } from "../../app/context";
import { backgroundFilter, cssUrl } from "../../app/theme";
import type { Background, ImageFile } from "../../api/types";
import "./forms.css";

/** Props of {@link BackgroundPicker}. */
export interface BackgroundPickerProps {
  /** The current background. */
  value: Background;
  /** Called with the fields that changed. */
  onChange(patch: Partial<Background>): void;
}

/** URL field, preview, upload and uploaded-image gallery. */
export function BackgroundPicker({ value, onChange }: BackgroundPickerProps) {
  const { api } = useApp();
  const id = useId();
  const fileRef = useRef<HTMLInputElement>(null);
  const [images, setImages] = useState<ImageFile[]>([]);
  const [uploading, setUploading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    api.listImages().then(
      (list) => alive && setImages(list),
      // The gallery is a convenience; the URL field still works without it.
      () => alive && setImages([]),
    );
    return () => {
      alive = false;
    };
  }, [api]);

  const upload = async (file: File) => {
    setUploading(true);
    try {
      const image = await api.uploadImage(file);
      setImages((list) => [...list.filter((i) => i.name !== image.name), image]);
      onChange({ url: image.url });
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setUploading(false);
      if (fileRef.current) fileRef.current.value = "";
    }
  };

  return (
    <div className="field bg-picker">
      <label htmlFor={`${id}-url`}>Image URL</label>
      <div
        className="bg-preview"
        data-testid="background-preview"
        aria-hidden
        style={
          value.url
            ? {
                backgroundImage: cssUrl(value.url),
                filter: backgroundFilter(value) || undefined,
                opacity: value.opacity,
              }
            : undefined
        }
      >
        {!value.url && <span>No image yet</span>}
      </div>
      <div className="icon-picker-row">
        <input
          id={`${id}-url`}
          type="text"
          spellCheck={false}
          placeholder="https://… or /images/wallpaper.jpg"
          aria-describedby={`${id}-help`}
          value={value.url}
          onChange={(e) => onChange({ url: e.target.value })}
        />
        <input
          ref={fileRef}
          type="file"
          accept="image/*"
          className="sr-only"
          aria-label="Upload background image file"
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
      <span id={`${id}-help`} className="field-help">
        Uploads are kept in the data folder under images/ and served at /images/. Files copied there
        from gethomepage&apos;s public/images work as they did before.
      </span>
      {error && (
        <span className="field-error" role="alert">
          {error}
        </span>
      )}
      {images.length > 0 && (
        <div className="bg-grid" role="group" aria-label="Uploaded images">
          {images.map((img) => (
            <button
              key={img.name}
              type="button"
              className={`btn bg-choice${img.url === value.url ? " selected" : ""}`}
              title={img.name}
              aria-label={`Use image ${img.name}`}
              aria-pressed={img.url === value.url}
              onClick={() => onChange({ url: img.url })}
            >
              <img src={img.url} alt="" loading="lazy" />
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
