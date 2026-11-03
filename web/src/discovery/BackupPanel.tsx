/**
 * Import from an existing gethomepage config folder, restore a homedit
 * backup, or download the config as YAML.
 */
import { useState, type FormEvent } from "react";
import type { ImportResult } from "../api/types";
import { useApp } from "../app/context";
import { loginErrorMessage } from "../auth/LoginPage";

/** File names the import understands. */
export const IMPORT_ACCEPT = ".yaml,.yml,.css";

/** The import / export panel shown in the editor drawer. */
export function BackupPanel({ onClose }: { onClose: () => void }) {
  const app = useApp();
  const [files, setFiles] = useState<File[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<ImportResult | null>(null);

  const upload = async (e: FormEvent) => {
    e.preventDefault();
    if (files.length === 0) return;
    if (
      app.dirty &&
      !window.confirm(
        "Importing replaces the current config. Unsaved changes will be lost. Continue?",
      )
    ) {
      return;
    }
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const res = await app.api.importFiles(files);
      setResult(res);
      await app.reload();
    } catch (err) {
      setError(loginErrorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div>
      <h2>Import and export</h2>
      <form onSubmit={upload}>
        <h3 className="panel-h3">Import</h3>
        <p className="field-help">
          Pick files from a gethomepage config folder (services.yaml, bookmarks.yaml, widgets.yaml,
          settings.yaml, docker.yaml, custom.css), or a homedit.yaml backup to restore. The current
          version stays in the history, so you can undo an import there.
        </p>
        <div className="field">
          <label htmlFor="import-files">Files</label>
          <input
            id="import-files"
            type="file"
            multiple
            accept={IMPORT_ACCEPT}
            onChange={(e) => setFiles(Array.from(e.target.files ?? []))}
          />
          {files.length > 0 && (
            <span className="field-help">{files.map((f) => f.name).join(", ")}</span>
          )}
        </div>
        <button type="submit" className="btn btn-primary" disabled={busy || files.length === 0}>
          {busy ? "Importing…" : "Import"}
        </button>
      </form>
      {error && (
        <div className="error-banner" role="alert" style={{ marginTop: 14 }}>
          {error}
        </div>
      )}
      {result && (
        <div role="status" style={{ marginTop: 14 }}>
          <p>
            Imported as version {result.version}.
            {result.warnings.length === 0 && " No problems found."}
          </p>
          {result.warnings.length > 0 && (
            <ul className="import-warnings">
              {result.warnings.map((w, i) => (
                <li key={i}>
                  <strong>{w.file}</strong>
                  {w.path && <code> {w.path}</code>}: {w.message}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      <h3 className="panel-h3">Export</h3>
      <p className="field-help">
        Download the whole config as homedit.yaml, for backups or to move to another server.
      </p>
      <div className="btn-row">
        <a className="btn" href={app.api.exportUrl(false)} download="homedit.yaml">
          Download (without secrets)
        </a>
        <a
          className="btn"
          href={app.api.exportUrl(true)}
          download="homedit.yaml"
          onClick={(e) => {
            if (
              !window.confirm(
                "The file will contain API keys and passwords in plain text. Keep it safe. Continue?",
              )
            ) {
              e.preventDefault();
            }
          }}
        >
          Download with secrets
        </a>
      </div>

      <div className="drawer-footer btn-row">
        <button type="button" className="btn" onClick={onClose}>
          Close
        </button>
      </div>
    </div>
  );
}
