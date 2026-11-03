-- Scheduled rescans (owned by the server's schedule module). One row.
CREATE TABLE scan_schedule (
    id             INTEGER PRIMARY KEY CHECK (id = 1),
    enabled        INTEGER NOT NULL DEFAULT 0,
    interval_hours INTEGER NOT NULL DEFAULT 24,
    request        TEXT    NOT NULL,   -- JSON: scanner::ScanRequest
    last_run_at    INTEGER,            -- ms since the epoch
    updated_by     TEXT
);
