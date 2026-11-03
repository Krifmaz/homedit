/**
 * Admin panel to list, add, change and remove user accounts.
 */
import { useCallback, useEffect, useState, type FormEvent } from "react";
import type { Role, UserAccount } from "../api/types";
import { useApp } from "../app/context";
import { loginErrorMessage, MIN_PASSWORD } from "./LoginPage";

/** The users panel shown in the editor drawer. */
export function UsersPanel({ onClose }: { onClose: () => void }) {
  const { api, me } = useApp();
  const [users, setUsers] = useState<UserAccount[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [role, setRole] = useState<Role>("viewer");

  const load = useCallback(async () => {
    try {
      setUsers(await api.users());
    } catch (e) {
      setError(loginErrorMessage(e));
    }
  }, [api]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- initial fetch
    void load();
  }, [load]);

  const run = async (fn: () => Promise<unknown>) => {
    setError(null);
    try {
      await fn();
      await load();
    } catch (e) {
      setError(loginErrorMessage(e));
    }
  };

  const add = (e: FormEvent) => {
    e.preventDefault();
    void run(async () => {
      await api.createUser({ username: name.trim(), password, role });
      setName("");
      setPassword("");
    });
  };

  return (
    <div>
      <h2>Users</h2>
      {error && (
        <div className="error-banner" role="alert">
          {error}
        </div>
      )}
      {!users && !error && <p className="field-help">Loading…</p>}
      {users && (
        <ul className="user-list">
          {users.map((u) => (
            <li key={u.id} className={u.disabled ? "is-disabled" : undefined}>
              <div className="user-main">
                <strong>{u.username}</strong>
                {u.id === me?.user?.id && <span className="badge">you</span>}
                <span className="badge">{u.source}</span>
                {u.disabled && <span className="badge">disabled</span>}
              </div>
              <div className="btn-row">
                <label className="sr-only" htmlFor={`role-${u.id}`}>
                  Role of {u.username}
                </label>
                <select
                  id={`role-${u.id}`}
                  value={u.role}
                  onChange={(e) =>
                    void run(() => api.updateUser(u.id, { role: e.target.value as Role }))
                  }
                >
                  <option value="viewer">Viewer</option>
                  <option value="admin">Admin</option>
                </select>
                <button
                  type="button"
                  className="btn"
                  onClick={() => void run(() => api.updateUser(u.id, { disabled: !u.disabled }))}
                >
                  {u.disabled ? "Enable" : "Disable"}
                </button>
                <button
                  type="button"
                  className="btn btn-danger"
                  aria-label={`Delete ${u.username}`}
                  onClick={() => {
                    if (window.confirm(`Delete user ${u.username}?`))
                      void run(() => api.deleteUser(u.id));
                  }}
                >
                  Delete
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
      <form onSubmit={add} className="user-add">
        <h3>Add a local user</h3>
        <div className="field">
          <label htmlFor="nu-name">Username</label>
          <input
            id="nu-name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            autoComplete="off"
          />
        </div>
        <div className="field">
          <label htmlFor="nu-pass">Password</label>
          <input
            id="nu-pass"
            type="password"
            autoComplete="new-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
          <span className="field-help">At least {MIN_PASSWORD} characters.</span>
        </div>
        <div className="field">
          <label htmlFor="nu-role">Role</label>
          <select id="nu-role" value={role} onChange={(e) => setRole(e.target.value as Role)}>
            <option value="viewer">Viewer (can only look)</option>
            <option value="admin">Admin (can edit and scan)</option>
          </select>
        </div>
        <button
          type="submit"
          className="btn btn-primary"
          disabled={!name.trim() || password.length < MIN_PASSWORD}
        >
          Add user
        </button>
      </form>
      <div className="drawer-footer btn-row">
        <button type="button" className="btn" onClick={onClose}>
          Close
        </button>
      </div>
    </div>
  );
}
