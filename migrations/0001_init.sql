-- oxide-fab schema. PostgreSQL is the single source of truth: every state
-- change is appended to lot_event and projected onto the entity tables in the
-- same transaction.

CREATE TABLE IF NOT EXISTS product (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    wafer_size_mm INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS route (
    id   TEXT PRIMARY KEY,
    name TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS route_step (
    route_id         TEXT NOT NULL REFERENCES route (id),
    step_no          INTEGER NOT NULL,
    name             TEXT NOT NULL,
    equipment_kind   TEXT NOT NULL,
    recipe_id        TEXT NOT NULL,
    duration_ms      BIGINT NOT NULL,
    target           DOUBLE PRECISION,
    ucl              DOUBLE PRECISION,
    lcl              DOUBLE PRECISION,
    PRIMARY KEY (route_id, step_no)
);

CREATE TABLE IF NOT EXISTS recipe (
    id       TEXT PRIMARY KEY,
    version  INTEGER NOT NULL,
    checksum TEXT NOT NULL,
    body     TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS equipment (
    id                   TEXT PRIMARY KEY,
    kind                 TEXT NOT NULL,
    state                TEXT NOT NULL,
    chamber              TEXT NOT NULL,
    qualification_matrix JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS lot (
    id            TEXT PRIMARY KEY,
    product_id    TEXT NOT NULL,
    route_id      TEXT NOT NULL,
    priority      INTEGER NOT NULL,
    state         TEXT NOT NULL,
    hold_reason   TEXT,
    current_step  INTEGER NOT NULL,
    wafer_count   INTEGER NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL,
    parent_lot_id TEXT
);

CREATE TABLE IF NOT EXISTS wafer (
    id           TEXT PRIMARY KEY,
    lot_id       TEXT NOT NULL REFERENCES lot (id),
    slot         INTEGER NOT NULL,
    state        TEXT NOT NULL,
    defect_info  TEXT,
    rework_count INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS wafer_lot_idx ON wafer (lot_id, slot);

-- Append-only. The only write entry point for state changes.
CREATE TABLE IF NOT EXISTS lot_event (
    seq            BIGSERIAL PRIMARY KEY,
    event_id       UUID NOT NULL UNIQUE,
    lot_id         TEXT NOT NULL,
    wafer_id       TEXT,
    kind           TEXT NOT NULL,
    payload        JSONB NOT NULL,
    occurred_at    TIMESTAMPTZ NOT NULL,
    occurred_at_ms BIGINT NOT NULL,
    actor          TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS lot_event_lot_idx ON lot_event (lot_id, seq);

-- Aggregate snapshots so a restart replays only the tail of the log.
CREATE TABLE IF NOT EXISTS snapshot (
    aggregate_id TEXT PRIMARY KEY,
    state        JSONB NOT NULL,
    last_seq     BIGINT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Transactional outbox: written in the same transaction as lot_event.
CREATE TABLE IF NOT EXISTS outbox (
    event_id     UUID PRIMARY KEY,
    aggregate_id TEXT NOT NULL,
    topic        TEXT NOT NULL,
    payload      JSONB NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    published_at TIMESTAMPTZ,
    attempts     INTEGER NOT NULL DEFAULT 0,
    last_error   TEXT
);

CREATE INDEX IF NOT EXISTS outbox_unpublished_idx
    ON outbox (created_at) WHERE published_at IS NULL;

-- Delivery-side dedupe ledger.
CREATE TABLE IF NOT EXISTS published_event (
    event_id     UUID PRIMARY KEY,
    store        TEXT NOT NULL,
    committed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS audit_log (
    id     BIGSERIAL PRIMARY KEY,
    who    TEXT NOT NULL,
    action TEXT NOT NULL,
    target TEXT NOT NULL,
    before JSONB,
    after  JSONB,
    at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Operator actions taken through the API are auditable.
CREATE INDEX IF NOT EXISTS audit_log_at_idx ON audit_log (at DESC);
