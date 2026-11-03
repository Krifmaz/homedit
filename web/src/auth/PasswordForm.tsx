/**
 * Drawer form for the signed-in local user to change their password.
 */
import { useState, type FormEvent } from "react";
import { useApp } from "../app/context";
import { loginErrorMessage, MIN_PASSWORD } from "./LoginPage";

/** Change-password form. */
export function PasswordForm({ onClose }: { onClose: () => void }) {
  const { api } = useApp();
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);
  const ok = current !== "" && next.length >= MIN_PASSWORD && next === confirm && !busy;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!ok) return;
    setBusy(true);
    setError(null);
    try {
      await api.changePassword(current, next);
      setDone(true);
    } catch (err) {
      setError(loginErrorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form onSubmit={submit}>
      <h2>Change password</h2>
      {done && (
        <p role="status" className="field-help">
          Password changed.
        </p>
      )}
      {error && (
        <div className="error-banner" role="alert">
          {error}
        </div>
      )}
      <div className="field">
        <label htmlFor="pw-current">Current password</label>
        <input
          id="pw-current"
          type="password"
          autoComplete="current-password"
          value={current}
          onChange={(e) => setCurrent(e.target.value)}
        />
      </div>
      <div className="field">
        <label htmlFor="pw-new">New password</label>
        <input
          id="pw-new"
          type="password"
          autoComplete="new-password"
          value={next}
          onChange={(e) => setNext(e.target.value)}
        />
        <span className={next && next.length < MIN_PASSWORD ? "field-error" : "field-help"}>
          At least {MIN_PASSWORD} characters.
        </span>
      </div>
      <div className="field">
        <label htmlFor="pw-confirm">Confirm new password</label>
        <input
          id="pw-confirm"
          type="password"
          autoComplete="new-password"
          value={confirm}
          onChange={(e) => setConfirm(e.target.value)}
        />
        {confirm && confirm !== next && <span className="field-error">Passwords don't match.</span>}
      </div>
      <div className="drawer-footer btn-row">
        <button type="submit" className="btn btn-primary" disabled={!ok}>
          {busy ? "Saving…" : "Change password"}
        </button>
        <button type="button" className="btn" onClick={onClose}>
          Close
        </button>
      </div>
    </form>
  );
}
