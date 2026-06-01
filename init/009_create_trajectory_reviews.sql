-- Migration 009: Human trajectory reviews for agent evaluation demos

CREATE TABLE IF NOT EXISTS trajectory_reviews (
    id UUID PRIMARY KEY,
    session_id UUID NOT NULL,
    reviewer VARCHAR(255) NOT NULL DEFAULT 'local-reviewer',
    overall_label VARCHAR(50) NOT NULL,
    failure_type VARCHAR(100),
    failure_event_id UUID,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_trajectory_reviews_session_id
    ON trajectory_reviews (session_id);

CREATE INDEX IF NOT EXISTS idx_trajectory_reviews_created_at
    ON trajectory_reviews (created_at DESC);

COMMENT ON TABLE trajectory_reviews IS
    'Human labels for full agent trajectories, used to export evaluation datasets.';
