/**
 * Header controls for the session: who is signed in, change password, sign
 * out, or a sign-in button for anonymous visitors when auth is enabled.
 */
import { useApp } from "../app/context";

/** Account controls shown in the header. */
export function AccountMenu() {
  const app = useApp();
  const me = app.me;
  if (!me?.authEnabled) return null;
  if (!me.user) {
    return (
      <button type="button" className="btn" onClick={() => app.requestLogin(true)}>
        Sign in
      </button>
    );
  }
  const local = !me.user.source || me.user.source === "local";
  return (
    <span className="account">
      <span
        className="account-name"
        title={`Signed in as ${me.user.name} (${me.role ?? "no role"})`}
      >
        {me.user.name}
      </span>
      {local && (
        <button
          type="button"
          className="btn btn-icon"
          onClick={() => app.open({ kind: "password" })}
        >
          Password
        </button>
      )}
      {me.role === "admin" && (
        <a className="btn btn-icon" href="/api/docs" target="_blank" rel="noopener noreferrer">
          API docs
        </a>
      )}
      <button type="button" className="btn" onClick={() => void app.logout()}>
        Sign out
      </button>
    </span>
  );
}
