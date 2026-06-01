-- Migration 004: Convert events to a TimescaleDB hypertable
--
-- This migration MUST run after 001_create_events.sql, which enables the
-- TimescaleDB extension used here.
--
-- The hosted demo keeps this migration to core hypertable creation only.
-- Compression and retention policies are intentionally omitted because some
-- managed TimescaleDB/PostgreSQL providers expose the extension without
-- enabling those license-gated features.

-- Enable the TimescaleDB extension (idempotent)
CREATE EXTENSION IF NOT EXISTS timescaledb CASCADE;

-- Convert the events table to a hypertable partitioned on timestamp.
-- migrate_data => true preserves any rows already in the table.
SELECT create_hypertable(
    'events',
    'timestamp',
    chunk_time_interval => INTERVAL '1 day',
    migrate_data        => true,
    if_not_exists       => true
);

COMMENT ON TABLE events IS
    'Core audit log (TimescaleDB hypertable): every agent request/response '
    'intercepted by the Agentland proxy. Partitioned by 1-day chunks, '
    'ready for trajectory review and evaluation workflows.';
