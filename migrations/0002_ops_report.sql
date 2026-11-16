-- Operational reports written by the scripts (backup / restore drills) are
-- shown by the dashboard. They live in PostgreSQL, next to the data they
-- describe, so the UI never reads files from a host it cannot see.

CREATE TABLE IF NOT EXISTS ops_report (
    kind        TEXT PRIMARY KEY,
    payload     JSONB NOT NULL,
    reported_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
