-- Port scan results (owned by the server's scan module).
CREATE TABLE scan_reports (
    id           INTEGER PRIMARY KEY,
    started_at   INTEGER NOT NULL,
    finished_at  INTEGER NOT NULL,
    requested_by TEXT,
    report       TEXT    NOT NULL   -- JSON: scanner::ScanReport
);
