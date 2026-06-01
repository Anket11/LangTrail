//! Human review persistence for trajectory-level agent evaluation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::db::StorePool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryReviewInput {
    pub reviewer: String,
    pub overall_label: String,
    pub failure_type: Option<String>,
    pub failure_event_id: Option<Uuid>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryReview {
    pub id: Uuid,
    pub session_id: Uuid,
    pub reviewer: String,
    pub overall_label: String,
    pub failure_type: Option<String>,
    pub failure_event_id: Option<Uuid>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub fn validate_label(label: &str) -> bool {
    matches!(label, "good" | "bad" | "needs_review")
}

pub async fn upsert_review(
    pool: &StorePool,
    session_id: Uuid,
    input: &TrajectoryReviewInput,
) -> Result<TrajectoryReview, sqlx::Error> {
    let row = sqlx::query(
        r#"
        INSERT INTO trajectory_reviews (
            id, session_id, reviewer, overall_label, failure_type,
            failure_event_id, notes, created_at, updated_at
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, NOW(), NOW()
        )
        ON CONFLICT (id) DO UPDATE SET
            reviewer = EXCLUDED.reviewer,
            overall_label = EXCLUDED.overall_label,
            failure_type = EXCLUDED.failure_type,
            failure_event_id = EXCLUDED.failure_event_id,
            notes = EXCLUDED.notes,
            updated_at = NOW()
        RETURNING id, session_id, reviewer, overall_label, failure_type,
                  failure_event_id, notes, created_at, updated_at
        "#,
    )
    .bind(Uuid::now_v7())
    .bind(session_id)
    .bind(&input.reviewer)
    .bind(&input.overall_label)
    .bind(&input.failure_type)
    .bind(input.failure_event_id)
    .bind(&input.notes)
    .fetch_one(pool)
    .await?;

    Ok(row_to_review(row))
}

pub async fn get_review_for_session(
    pool: &StorePool,
    session_id: Uuid,
) -> Result<Option<TrajectoryReview>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT id, session_id, reviewer, overall_label, failure_type,
               failure_event_id, notes, created_at, updated_at
        FROM trajectory_reviews
        WHERE session_id = $1
        ORDER BY updated_at DESC
        LIMIT 1
        "#,
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(row_to_review))
}

pub async fn list_reviewed_sessions(
    pool: &StorePool,
    limit: i64,
) -> Result<Vec<serde_json::Value>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT
            r.id AS review_id,
            r.session_id,
            r.reviewer,
            r.overall_label,
            r.failure_type,
            r.failure_event_id,
            r.notes,
            r.created_at,
            r.updated_at,
            MIN(e.timestamp) AS started_at,
            MAX(e.timestamp) AS ended_at,
            COUNT(e.id) AS event_count,
            MAX(e.agent_id) AS agent_id,
            MAX(e.model) AS model,
            COALESCE(SUM(e.total_tokens), 0) AS total_tokens,
            COALESCE(SUM(e.cost_usd), 0)::float8 AS total_cost_usd
        FROM trajectory_reviews r
        JOIN events e ON e.session_id = r.session_id
        GROUP BY r.id
        ORDER BY r.updated_at DESC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "review_id": r.try_get::<Uuid, _>("review_id").ok().map(|v| v.to_string()),
                "session_id": r.try_get::<Uuid, _>("session_id").ok().map(|v| v.to_string()),
                "reviewer": r.try_get::<String, _>("reviewer").ok(),
                "overall_label": r.try_get::<String, _>("overall_label").ok(),
                "failure_type": r.try_get::<Option<String>, _>("failure_type").ok().flatten(),
                "failure_event_id": r.try_get::<Option<Uuid>, _>("failure_event_id").ok().flatten().map(|v| v.to_string()),
                "notes": r.try_get::<Option<String>, _>("notes").ok().flatten(),
                "started_at": r.try_get::<DateTime<Utc>, _>("started_at").ok().map(|v| v.to_rfc3339()),
                "ended_at": r.try_get::<DateTime<Utc>, _>("ended_at").ok().map(|v| v.to_rfc3339()),
                "event_count": r.try_get::<i64, _>("event_count").ok(),
                "agent_id": r.try_get::<String, _>("agent_id").ok(),
                "model": r.try_get::<Option<String>, _>("model").ok().flatten(),
                "total_tokens": r.try_get::<i64, _>("total_tokens").ok(),
                "total_cost_usd": r.try_get::<f64, _>("total_cost_usd").ok(),
            })
        })
        .collect())
}

pub async fn list_recent_trajectories(
    pool: &StorePool,
    limit: i64,
) -> Result<Vec<serde_json::Value>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT
            e.session_id,
            MIN(e.timestamp) AS started_at,
            MAX(e.timestamp) AS ended_at,
            COUNT(*) AS event_count,
            MAX(e.agent_id) AS agent_id,
            MAX(e.model) AS model,
            COALESCE(SUM(e.total_tokens), 0) AS total_tokens,
            COALESCE(SUM(e.cost_usd), 0)::float8 AS total_cost_usd,
            r.overall_label,
            r.failure_type,
            r.failure_event_id,
            r.notes
        FROM events e
        LEFT JOIN LATERAL (
            SELECT overall_label, failure_type, failure_event_id, notes
            FROM trajectory_reviews
            WHERE session_id = e.session_id
            ORDER BY updated_at DESC
            LIMIT 1
        ) r ON true
        GROUP BY e.session_id, r.overall_label, r.failure_type, r.failure_event_id, r.notes
        ORDER BY MAX(e.timestamp) DESC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "session_id": r.try_get::<Uuid, _>("session_id").ok().map(|v| v.to_string()),
                "started_at": r.try_get::<DateTime<Utc>, _>("started_at").ok().map(|v| v.to_rfc3339()),
                "ended_at": r.try_get::<DateTime<Utc>, _>("ended_at").ok().map(|v| v.to_rfc3339()),
                "event_count": r.try_get::<i64, _>("event_count").ok(),
                "agent_id": r.try_get::<String, _>("agent_id").ok(),
                "model": r.try_get::<Option<String>, _>("model").ok().flatten(),
                "total_tokens": r.try_get::<i64, _>("total_tokens").ok(),
                "total_cost_usd": r.try_get::<f64, _>("total_cost_usd").ok(),
                "overall_label": r.try_get::<Option<String>, _>("overall_label").ok().flatten(),
                "failure_type": r.try_get::<Option<String>, _>("failure_type").ok().flatten(),
                "failure_event_id": r.try_get::<Option<Uuid>, _>("failure_event_id").ok().flatten().map(|v| v.to_string()),
                "notes": r.try_get::<Option<String>, _>("notes").ok().flatten(),
            })
        })
        .collect())
}

fn row_to_review(row: sqlx::postgres::PgRow) -> TrajectoryReview {
    TrajectoryReview {
        id: row.try_get("id").expect("id"),
        session_id: row.try_get("session_id").expect("session_id"),
        reviewer: row.try_get("reviewer").expect("reviewer"),
        overall_label: row.try_get("overall_label").expect("overall_label"),
        failure_type: row.try_get("failure_type").ok().flatten(),
        failure_event_id: row.try_get("failure_event_id").ok().flatten(),
        notes: row.try_get("notes").ok().flatten(),
        created_at: row.try_get("created_at").expect("created_at"),
        updated_at: row.try_get("updated_at").expect("updated_at"),
    }
}

#[cfg(test)]
mod tests {
    use super::validate_label;

    #[test]
    fn validate_label_accepts_review_labels() {
        assert!(validate_label("good"));
        assert!(validate_label("bad"));
        assert!(validate_label("needs_review"));
    }

    #[test]
    fn validate_label_rejects_unknown_values() {
        assert!(!validate_label("unsafe"));
        assert!(!validate_label(""));
    }
}
