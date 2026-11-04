/**
 * Network scan panel: pick targets and ports, watch progress, then add what
 * was found to the dashboard with the URL and widget type filled in.
 *
 * Only private ranges are accepted unless the server allows more. Scans are
 * plain TCP connects, rate limited and logged on the server.
 */
import { useCallback, useEffect, useState, type FormEvent } from "react";
import type { PortSet, ScanJob, ScanRequest, ScanSchedule } from "../api/types";
import { useApp } from "../app/context";
import { loginErrorMessage } from "../auth/LoginPage";
import { allGroups, newId, upsertGroup, upsertService } from "../editor/ops";
import {
  bareNetworks,
  foundServices,
  parseTargets,
  scanProgress,
  suggestionToService,
  widenBareNetworks,
  type FoundService,
} from "./suggest";

/** Human text for a rescan interval in hours. Exported for tests. */
export function intervalLabel(hours: number): string {
  if (hours % 168 === 0) return hours === 168 ? "week" : `${hours / 168} weeks`;
  if (hours % 24 === 0) return hours === 24 ? "day" : `${hours / 24} days`;
  return hours === 1 ? "hour" : `${hours} hours`;
}

/**
 * "Last run … Next run …" text. A schedule that never ran is due right away
 * (the server sends `nextRunAt: 0`). Exported for tests.
 */
export function scheduleTimes(s: ScanSchedule, now: number): string {
  const parts: string[] = [];
  if (s.lastRunAt) parts.push(`Last run ${new Date(s.lastRunAt).toLocaleString()}.`);
  if (s.enabled && s.nextRunAt !== undefined && s.nextRunAt !== null) {
    parts.push(
      s.nextRunAt <= now
        ? "Next run within a minute."
        : `Next run ${new Date(s.nextRunAt).toLocaleString()}.`,
    );
  }
  return parts.join(" ");
}

/** Poll interval while a scan runs. */
const POLL_MS = 1000;

/** The scan panel shown in the editor drawer. */
export function ScanPanel({ onClose }: { onClose: () => void }) {
  const app = useApp();
  const { api } = app;
  const [targets, setTargets] = useState("192.168.1.0/24");
  const [portKind, setPortKind] = useState<PortSet["kind"]>("common");
  const [portList, setPortList] = useState("80,443,8000-9000");
  const [fingerprint, setFingerprint] = useState(true);
  const [job, setJob] = useState<ScanJob | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [hideKnown, setHideKnown] = useState(true);
  const groups = app.doc ? allGroups(app.doc) : [];
  const [groupId, setGroupId] = useState<string>("");

  const refresh = useCallback(async () => {
    try {
      setJob(await api.scanJob());
    } catch (e) {
      setError(loginErrorMessage(e));
    }
  }, [api]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- initial fetch
    void refresh();
  }, [refresh]);

  const running = !!job?.running;
  useEffect(() => {
    if (!running) return;
    const t = setInterval(() => void refresh(), POLL_MS);
    return () => clearInterval(t);
  }, [running, refresh]);

  const [schedule, setSchedule] = useState<ScanSchedule | null>(null);
  const [scheduleMsg, setScheduleMsg] = useState<string | null>(null);
  /** When the schedule was last read or saved, for "next run" wording. */
  const [scheduleAt, setScheduleAt] = useState(() => Date.now());

  useEffect(() => {
    let alive = true;
    api.scanSchedule().then(
      (s) => {
        if (!alive) return;
        setSchedule(s);
        // Start from the scheduled request, so the form shows what rescans scan.
        if (s.request.targets.length > 0) {
          setTargets(s.request.targets.join("\n"));
          setPortKind(s.request.ports.kind);
          if (s.request.ports.kind === "list") setPortList(s.request.ports.ports);
          setFingerprint(s.request.fingerprint);
        }
      },
      () => {
        /* older servers have no schedule; the section stays hidden */
      },
    );
    return () => {
      alive = false;
    };
  }, [api]);

  /** The scan request the form describes. */
  const currentRequest = (): ScanRequest => ({
    targets: parseTargets(targets),
    ports: portKind === "list" ? { kind: "list", ports: portList } : { kind: portKind },
    fingerprint,
  });

  const saveSchedule = async (next: Partial<ScanSchedule>) => {
    if (!schedule) return;
    setScheduleMsg(null);
    try {
      const saved = await api.saveScanSchedule({ ...schedule, request: currentRequest(), ...next });
      setSchedule(saved);
      setScheduleAt(Date.now());
      setScheduleMsg("Schedule saved.");
    } catch (err) {
      setScheduleMsg(loginErrorMessage(err));
    }
  };

  const start = async (e: FormEvent) => {
    e.preventDefault();
    const req = currentRequest();
    if (req.targets.length === 0) return;
    if (portKind === "full" && !window.confirm("Scanning all 65,535 ports is slow. Continue?"))
      return;
    setError(null);
    try {
      await api.startScan(req);
      await refresh();
    } catch (err) {
      setError(loginErrorMessage(err));
    }
  };

  const add = (f: FoundService) => {
    if (!f.suggestion || !app.doc) return;
    let doc = app.doc;
    let gid = groupId || groups[0]?.id;
    if (!gid) {
      gid = newId("g");
      doc = upsertGroup(doc, {
        id: gid,
        name: "Discovered",
        hidden: false,
        layout: { columns: 3, style: "grid" },
      });
    }
    const svc = suggestionToService(f.suggestion, newId("s"));
    app.apply(upsertService(doc, gid, svc));
    // Open the card's form so the admin can add the API key and test it.
    app.open({ kind: "service", groupId: gid, serviceId: svc.id });
  };

  const bare = bareNetworks(targets);
  const progress = job ? scanProgress(job) : null;
  const found = job ? foundServices(job) : [];
  const shown = hideKnown ? found.filter((f) => !f.onDashboard) : found;
  const known = found.length - found.filter((f) => !f.onDashboard).length;

  return (
    <div className="scan">
      <h2>Find services</h2>
      <form onSubmit={start}>
        <div className="field">
          <label htmlFor="scan-targets">Networks and hosts</label>
          <textarea
            id="scan-targets"
            className="scan-targets"
            value={targets}
            onChange={(e) => setTargets(e.target.value)}
            aria-describedby="scan-targets-help"
          />
          <span id="scan-targets-help" className="field-help">
            One per line: a CIDR range (192.168.1.0/24), an address, or a hostname. Only private
            ranges are allowed unless the server is configured otherwise.
          </span>
          {bare.length > 0 && (
            <div className="scan-hint" role="status">
              <span>
                {bare.length === 1
                  ? `${bare[0]} is a network address, so only that one address is scanned. To scan the whole network, use ${bare[0]}/24.`
                  : `${bare.join(", ")} are network addresses, so only those addresses are scanned. To scan the whole networks, add /24 to each.`}
              </span>
              <button
                type="button"
                className="btn"
                onClick={() => setTargets(widenBareNetworks(targets))}
              >
                Scan {bare.length === 1 ? `${bare[0]}/24` : "the /24 ranges"}
              </button>
            </div>
          )}
        </div>
        <div className="field">
          <label htmlFor="scan-ports">Ports</label>
          <select
            id="scan-ports"
            value={portKind}
            onChange={(e) => setPortKind(e.target.value as PortSet["kind"])}
          >
            <option value="common">Common web ports and known apps (default)</option>
            <option value="catalogue">Known apps only (fastest)</option>
            <option value="list">Custom list</option>
            <option value="full">All ports 1–65535 (slow)</option>
          </select>
        </div>
        {portKind === "list" && (
          <div className="field">
            <label htmlFor="scan-list">Port list</label>
            <input id="scan-list" value={portList} onChange={(e) => setPortList(e.target.value)} />
            <span className="field-help">Comma separated, ranges allowed: 80,443,8000-8100</span>
          </div>
        )}
        <div className="field field-inline">
          <input
            id="scan-fp"
            type="checkbox"
            checked={fingerprint}
            onChange={(e) => setFingerprint(e.target.checked)}
          />
          <label htmlFor="scan-fp">Identify apps (page title, favicon, known API paths)</label>
        </div>
        <button
          type="submit"
          className="btn btn-primary"
          disabled={running || parseTargets(targets).length === 0}
        >
          {running ? "Scanning…" : "Start scan"}
        </button>
      </form>

      {error && (
        <div className="error-banner" role="alert" style={{ marginTop: 14 }}>
          {error}
        </div>
      )}
      {job?.error && (
        <div className="error-banner" role="alert" style={{ marginTop: 14 }}>
          Scan failed: {job.error}
        </div>
      )}

      {running && (
        <div className="scan-progress" role="status">
          <progress max={1} value={progress ?? undefined} aria-label="Scan progress" />
          <span className="field-help">
            {progress === null ? "Starting…" : `${Math.round(progress * 100)}%`}
          </span>
        </div>
      )}

      {job?.diff &&
        !running &&
        job.diff.appeared.length + job.diff.disappeared.length + job.diff.changed.length > 0 && (
          <section className="scan-results" aria-label="Changes since the last scan">
            <h3>Since the last scan</h3>
            <ul className="scan-list scan-diff">
              {job.diff.appeared.map((c) => {
                const f = found.find((x) => x.key === `${c.ip}:${c.service.port}`);
                return (
                  <li key={`a-${c.ip}:${c.service.port}`} className="appeared">
                    <strong>{f?.name ?? c.service.app?.name ?? `Port ${c.service.port}`}</strong>
                    <span className="field-help">
                      New on {c.hostname || c.ip}:{c.service.port}
                    </span>
                    {f?.suggestion && !f.onDashboard && (
                      <button type="button" className="btn" onClick={() => add(f)}>
                        + Add to dashboard
                      </button>
                    )}
                  </li>
                );
              })}
              {job.diff.disappeared.map((c) => (
                <li key={`d-${c.ip}:${c.service.port}`} className="disappeared">
                  <strong>
                    {c.service.app?.name ?? c.service.http?.title ?? `Port ${c.service.port}`}
                  </strong>
                  <span className="field-help">
                    No longer answers on {c.hostname || c.ip}:{c.service.port}
                    {c.service.on_dashboard && " (it has a card on the dashboard)"}
                  </span>
                </li>
              ))}
              {job.diff.changed.map((c) => (
                <li key={`c-${c.ip}:${c.service.port}`}>
                  <strong>
                    {c.service.app?.name ?? c.service.http?.title ?? `Port ${c.service.port}`}
                  </strong>
                  <span className="field-help">
                    Changed on {c.hostname || c.ip}:{c.service.port}
                    {c.service.app?.version && ` (now v${c.service.app.version})`}
                  </span>
                </li>
              ))}
            </ul>
          </section>
        )}

      {job?.report && !running && (
        <section className="scan-results" aria-label="Scan results">
          <h3>
            Found {found.length} open {found.length === 1 ? "port" : "ports"}
            {known > 0 && <span className="field-help"> · {known} already on the dashboard</span>}
          </h3>
          <div className="btn-row" style={{ alignItems: "center", marginBottom: 10 }}>
            <label className="field-inline" style={{ display: "flex", gap: 6 }}>
              <input
                type="checkbox"
                checked={hideKnown}
                onChange={(e) => setHideKnown(e.target.checked)}
              />
              Hide ones already on the dashboard
            </label>
          </div>
          {groups.length > 0 && (
            <div className="field">
              <label htmlFor="scan-group">Add to group</label>
              <select
                id="scan-group"
                value={groupId || groups[0]!.id}
                onChange={(e) => setGroupId(e.target.value)}
              >
                {groups.map((g) => (
                  <option key={g.id} value={g.id}>
                    {g.name}
                  </option>
                ))}
              </select>
            </div>
          )}
          {shown.length === 0 && <p className="field-help">Nothing new found.</p>}
          <ul className="scan-list">
            {shown.map((f) => (
              <li key={f.key}>
                <div className="scan-item-main">
                  <strong>{f.name}</strong>
                  {f.widget && <span className="badge">widget: {f.widget}</span>}
                  {f.onDashboard && <span className="badge">on dashboard</span>}
                </div>
                <div className="field-help">
                  {f.href ? (
                    <a href={f.href} target="_blank" rel="noopener noreferrer">
                      {f.href}
                    </a>
                  ) : (
                    `${f.host}:${f.port}`
                  )}
                  {!f.appId && " · not recognised"}
                </div>
                {f.suggestion && (
                  <button
                    type="button"
                    className="btn"
                    onClick={() => add(f)}
                    aria-label={`Add ${f.name} on ${f.host}:${f.port} to the dashboard`}
                  >
                    + Add to dashboard
                  </button>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}

      {schedule && (
        <section className="scan-schedule" aria-label="Rescan schedule">
          <h3 className="panel-h3">Rescan automatically</h3>
          <p className="field-help">
            Rescans use the networks and ports above and tell admins what appeared or disappeared.
          </p>
          <div className="field field-inline">
            <input
              id="sched-on"
              type="checkbox"
              checked={schedule.enabled}
              onChange={(e) => setSchedule({ ...schedule, enabled: e.target.checked })}
            />
            <label htmlFor="sched-on">Rescan on a schedule</label>
          </div>
          <div className="field">
            <label htmlFor="sched-every">Every</label>
            <select
              id="sched-every"
              value={schedule.intervalHours}
              onChange={(e) => setSchedule({ ...schedule, intervalHours: Number(e.target.value) })}
            >
              {[...new Set([1, 6, 12, 24, 72, 168, 720, schedule.intervalHours])]
                .sort((a, b) => a - b)
                .map((h) => (
                  <option key={h} value={h}>
                    {intervalLabel(h)}
                  </option>
                ))}
            </select>
          </div>
          {(schedule.lastRunAt || schedule.enabled) && (
            <p className="field-help">{scheduleTimes(schedule, scheduleAt)}</p>
          )}
          <button
            type="button"
            className="btn"
            onClick={() => void saveSchedule({})}
            disabled={schedule.enabled && parseTargets(targets).length === 0}
          >
            Save schedule
          </button>
          {scheduleMsg && (
            <p className="field-help" role="status">
              {scheduleMsg}
            </p>
          )}
        </section>
      )}

      <div className="drawer-footer btn-row">
        <button type="button" className="btn" onClick={onClose}>
          Close
        </button>
      </div>
    </div>
  );
}
