/**
 * Sign-in page, and the first-run screen that creates the first admin.
 *
 * Shown instead of the dashboard when the server requires a login (role
 * `null` with auth enabled, or a 401 from the dashboard) or when no users
 * exist yet (`setupRequired`).
 */
import { useState, type FormEvent } from "react";
import { ApiError } from "../api/client";
import { useApp } from "../app/context";

/** Minimum password length the server accepts for new passwords. */
export const MIN_PASSWORD = 10;

/** Messages for `?authError=<code>` after a failed OIDC round trip. */
const AUTH_ERRORS: Record<string, string> = {
  access_denied: "Sign-in was cancelled or denied.",
  disabled: "This account is disabled.",
  no_account: "No account is linked to that identity.",
};

/** Human text for an `authError` query parameter. Exported for tests. */
export function authErrorMessage(code: string | null): string | null {
  if (!code) return null;
  return AUTH_ERRORS[code] ?? `Single sign-on failed (${code}).`;
}

/** Human text for a failed login or setup call. Exported for tests. */
export function loginErrorMessage(e: unknown): string {
  if (!(e instanceof ApiError)) return e instanceof Error ? e.message : String(e);
  switch (e.code) {
    case "invalid_credentials":
      return "Wrong username or password.";
    case "rate_limited":
      return e.retryAfter
        ? `Too many attempts. Try again in ${Math.ceil(e.retryAfter / 60)} min.`
        : "Too many attempts. Try again later.";
    case "disabled":
      return "Password sign-in is turned off on this server.";
    default:
      return e.message || "Sign-in failed.";
  }
}

/** The login or setup screen. */
export function LoginPage() {
  const { api, me, refreshSession, loginOptional, requestLogin } = useApp();
  const setup = !!me?.setupRequired;
  const methods = me?.methods ?? { local: true, oidc: null, proxy: false };
  const [username, setUsername] = useState(setup ? "admin" : "");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(() =>
    authErrorMessage(new URLSearchParams(window.location.search).get("authError")),
  );

  const tooShort = setup && password.length > 0 && password.length < MIN_PASSWORD;
  const mismatch = setup && confirm.length > 0 && confirm !== password;
  const canSubmit =
    !busy &&
    username.trim() !== "" &&
    password !== "" &&
    (!setup || (password.length >= MIN_PASSWORD && confirm === password));

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;
    setBusy(true);
    setError(null);
    try {
      if (setup) await api.setup(username.trim(), password);
      else await api.login(username.trim(), password);
      refreshSession();
    } catch (err) {
      setError(loginErrorMessage(err));
      setBusy(false);
    }
  };

  const returnTo =
    window.location.pathname + window.location.search.replace(/[?&]authError=[^&]*/, "");
  return (
    <main className="login">
      <form className="login-card" onSubmit={submit} aria-labelledby="login-title">
        <h1 id="login-title">{setup ? "Create the admin account" : "Sign in"}</h1>
        {setup && (
          <p className="field-help">
            No users exist yet. The account you create here becomes the first admin.
          </p>
        )}
        {error && (
          <div className="error-banner" role="alert">
            {error}
          </div>
        )}
        {(setup || methods.local) && (
          <>
            <div className="field">
              <label htmlFor="login-user">Username</label>
              <input
                id="login-user"
                autoComplete="username"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                autoFocus
              />
            </div>
            <div className="field">
              <label htmlFor="login-pass">Password</label>
              <input
                id="login-pass"
                type="password"
                autoComplete={setup ? "new-password" : "current-password"}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                aria-invalid={tooShort || undefined}
                aria-describedby={setup ? "login-pass-help" : undefined}
              />
              {setup && (
                <span id="login-pass-help" className={tooShort ? "field-error" : "field-help"}>
                  At least {MIN_PASSWORD} characters.
                </span>
              )}
            </div>
            {setup && (
              <div className="field">
                <label htmlFor="login-confirm">Confirm password</label>
                <input
                  id="login-confirm"
                  type="password"
                  autoComplete="new-password"
                  value={confirm}
                  onChange={(e) => setConfirm(e.target.value)}
                  aria-invalid={mismatch || undefined}
                />
                {mismatch && <span className="field-error">Passwords don't match.</span>}
              </div>
            )}
            <button type="submit" className="btn btn-primary login-submit" disabled={!canSubmit}>
              {busy ? "Please wait…" : setup ? "Create admin and sign in" : "Sign in"}
            </button>
          </>
        )}
        {!setup && methods.oidc && (
          <>
            {methods.local && <div className="login-or">or</div>}
            <a
              className="btn login-submit"
              href={`/api/auth/oidc/login?returnTo=${encodeURIComponent(returnTo || "/")}`}
            >
              {methods.oidc}
            </a>
          </>
        )}
        {!setup && !methods.local && !methods.oidc && (
          <p className="field-help">
            Sign-in is handled by your reverse proxy. Reload the page after signing in there.
          </p>
        )}
        {loginOptional && (
          <button
            type="button"
            className="btn btn-icon login-back"
            onClick={() => requestLogin(false)}
          >
            ← Back to the dashboard
          </button>
        )}
      </form>
    </main>
  );
}
