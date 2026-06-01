//! Trajectory review handlers for human-in-the-loop agent evaluation.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ListTrajectoriesParams {
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SaveReviewBody {
    pub reviewer: Option<String>,
    pub overall_label: String,
    pub failure_type: Option<String>,
    pub failure_event_id: Option<String>,
    pub notes: Option<String>,
}

pub async fn list_trajectories(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListTrajectoriesParams>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(25).clamp(1, 100);

    match agentland_store::reviews::list_recent_trajectories(&state.pool, limit).await {
        Ok(data) => (StatusCode::OK, Json(json!({ "data": data, "total": data.len() }))),
        Err(e) => {
            tracing::error!("list_trajectories store error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "failed to list trajectories", "detail": e.to_string() })),
            )
        }
    }
}

pub async fn get_trajectory(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&session_id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "invalid session_id" })),
            )
        }
    };

    let events = match agentland_store::events::get_session_events(&state.pool, uuid).await {
        Ok(events) => events,
        Err(e) => {
            tracing::error!("get_trajectory events error: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "failed to fetch trajectory", "detail": e.to_string() })),
            );
        }
    };

    let review = match agentland_store::reviews::get_review_for_session(&state.pool, uuid).await {
        Ok(review) => review,
        Err(e) => {
            tracing::error!("get_trajectory review error: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "failed to fetch review", "detail": e.to_string() })),
            );
        }
    };

    (
        StatusCode::OK,
        Json(json!({
            "data": {
                "session_id": session_id,
                "events": events,
                "steps": build_logical_steps(&events),
                "review": review,
            }
        })),
    )
}

pub async fn save_review(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(body): Json<SaveReviewBody>,
) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&session_id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "invalid session_id" })),
            )
        }
    };

    if !agentland_store::reviews::validate_label(&body.overall_label) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "overall_label must be good, bad, or needs_review" })),
        );
    }

    let failure_event_id = match body.failure_event_id.as_deref() {
        Some("") | None => None,
        Some(id) => match Uuid::parse_str(id) {
            Ok(id) => Some(id),
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": "invalid failure_event_id" })),
                )
            }
        },
    };

    let input = agentland_store::reviews::TrajectoryReviewInput {
        reviewer: body
            .reviewer
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "local-reviewer".to_string()),
        overall_label: body.overall_label,
        failure_type: body.failure_type.filter(|s| !s.trim().is_empty()),
        failure_event_id,
        notes: body.notes.filter(|s| !s.trim().is_empty()),
    };

    match agentland_store::reviews::upsert_review(&state.pool, uuid, &input).await {
        Ok(review) => (StatusCode::OK, Json(json!({ "data": review }))),
        Err(e) => {
            tracing::error!("save_review store error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "failed to save review", "detail": e.to_string() })),
            )
        }
    }
}

pub async fn assist_review(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&session_id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "invalid session_id" })),
            )
        }
    };

    let api_key = std::env::var("AGENTLAND_REVIEW_ASSIST_OPENAI_API_KEY")
        .or_else(|_| std::env::var("OPENAI_API_KEY"))
        .unwrap_or_default();
    if api_key.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "review assistant is not configured",
                "detail": "set AGENTLAND_REVIEW_ASSIST_OPENAI_API_KEY or OPENAI_API_KEY on the proxy service"
            })),
        );
    }

    let events = match agentland_store::events::get_session_events(&state.pool, uuid).await {
        Ok(events) => events,
        Err(e) => {
            tracing::error!("assist_review events error: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "failed to fetch trajectory", "detail": e.to_string() })),
            );
        }
    };

    let steps = build_logical_steps(&events);
    let summary = build_assist_summary(&steps);
    let model = std::env::var("AGENTLAND_REVIEW_ASSIST_MODEL")
        .unwrap_or_else(|_| "gpt-4o-mini".to_string());
    let endpoint = std::env::var("AGENTLAND_REVIEW_ASSIST_OPENAI_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1/chat/completions".to_string());

    let body = json!({
        "model": model,
        "temperature": 0,
        "max_tokens": 260,
        "response_format": { "type": "json_object" },
        "messages": [
            {
                "role": "system",
                "content": "You assist human reviewers evaluating agent trajectories. Return only valid JSON. Do not make the final decision; provide suggestions for a human reviewer."
            },
            {
                "role": "user",
                "content": format!(
                    "Review this agent trajectory and suggest a human review label.\n\nAllowed labels: good, bad, needs_review.\nAllowed failure_type values: bad_answer, bad_tool_use, hallucination, inefficient, unsafe, other, or null.\nReturn JSON with keys: suggested_label, confidence, failure_type, failure_step_index, critique, quality_signals.\nfailure_step_index must be a 1-based integer from the visible Step list, or null if no specific step caused the issue.\n\nStep selection rubric:\n- If the agent made an unnecessary, incorrect, or inefficient tool-use decision, use failure_type bad_tool_use or inefficient and mark the model step that requested the tool.\n- Do not mark the tool execution step unless the tool result itself is wrong.\n- Mark the final model step only when the final answer is wrong, unsafe, or hallucinated independent of the tool choice.\nquality_signals must be an array of short snake_case strings.\n\nTrajectory:\n{}",
                    summary
                )
            }
        ]
    });

    let client = reqwest::Client::new();
    let response = match client
        .post(endpoint)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(e) => {
            tracing::warn!("review assistant request failed: {}", e);
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": "review assistant request failed", "detail": e.to_string() })),
            );
        }
    };

    let status = response.status();
    let value = match response.json::<Value>().await {
        Ok(value) => value,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": "invalid review assistant response", "detail": e.to_string() })),
            );
        }
    };

    if !status.is_success() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": "review assistant provider error", "detail": value })),
        );
    }

    let content = value
        .get("choices")
        .and_then(|v| v.get(0))
        .and_then(|v| v.get("message"))
        .and_then(|v| v.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("{}");

    let mut assist = parse_assist_response(content);
    normalize_prime_tool_review(&mut assist, &steps);
    let failure_step_index = assist
        .get("failure_step_index")
        .and_then(|v| v.as_u64())
        .and_then(|v| usize::try_from(v).ok())
        .filter(|index| *index >= 1 && *index <= steps.len())
        .or_else(|| fallback_failure_step_index(
            assist.get("suggested_label").and_then(|v| v.as_str()).unwrap_or("needs_review"),
            assist.get("failure_type").and_then(|v| v.as_str()),
            &steps,
        ));
    let failure_event_id = failure_step_index
        .and_then(|index| steps.get(index.saturating_sub(1)))
        .and_then(primary_step_event_id);

    (
        StatusCode::OK,
        Json(json!({
            "data": {
                "suggested_label": normalize_label(
                    assist.get("suggested_label").and_then(|v| v.as_str()).unwrap_or("needs_review")
                ),
                "confidence": assist.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0).clamp(0.0, 1.0),
                "failure_type": normalize_failure_type(assist.get("failure_type").and_then(|v| v.as_str())),
                "failure_step_index": failure_step_index,
                "failure_event_id": failure_event_id,
                "critique": assist.get("critique").and_then(|v| v.as_str()).unwrap_or(content),
                "quality_signals": coerce_quality_signals(assist.get("quality_signals")),
                "model": model,
            }
        })),
    )
}

pub async fn export_reviews(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let rows = match agentland_store::reviews::list_reviewed_sessions(&state.pool, 500).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("export_reviews store error: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "application/json")],
                json!({ "error": "failed to export reviews", "detail": e.to_string() }).to_string(),
            );
        }
    };

    let mut body = String::new();
    for row in rows {
        let mut raw_events = Vec::new();
        let mut steps = Vec::new();
        if let Some(session_id) = row.get("session_id").and_then(|v| v.as_str()) {
            if let Ok(uuid) = Uuid::parse_str(session_id) {
                if let Ok(events) = agentland_store::events::get_session_events(&state.pool, uuid).await {
                    steps = build_logical_steps(&events);
                    raw_events = events;
                }
            }
        }

        let labelbox_row = json!({
            "global_key": format!(
                "agentland-trajectory-{}",
                row.get("session_id").and_then(|v| v.as_str()).unwrap_or("unknown")
            ),
            "row_data": build_row_data(&row, &steps),
            "metadata_fields": [
                { "name": "agent_id", "value": row.get("agent_id").cloned().unwrap_or(json!("unknown")) },
                { "name": "session_id", "value": row.get("session_id").cloned().unwrap_or(json!("unknown")) },
                { "name": "model", "value": row.get("model").cloned().unwrap_or(json!("unknown")) },
                { "name": "event_count", "value": row.get("event_count").cloned().unwrap_or(json!(0)) },
                { "name": "total_tokens", "value": row.get("total_tokens").cloned().unwrap_or(json!(0)) },
                { "name": "total_cost_usd", "value": row.get("total_cost_usd").cloned().unwrap_or(json!(0.0)) },
                { "name": "overall_label", "value": row.get("overall_label").cloned().unwrap_or(json!("unlabeled")) },
                { "name": "failure_type", "value": row.get("failure_type").cloned().unwrap_or(json!("none")) }
            ],
            "agentland": {
                "review": row,
                "trajectory": {
                    "raw_events": raw_events,
                    "steps": steps
                }
            }
        });

        body.push_str(&labelbox_row.to_string());
        body.push('\n');
    }

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/x-ndjson")],
        body,
    )
}

fn build_row_data(review: &serde_json::Value, steps: &[serde_json::Value]) -> String {
    let mut text = String::new();
    text.push_str("Agent trajectory review\n\n");
    text.push_str(&format!(
        "Agent: {}\nSession: {}\nModel: {}\nLabel: {}\nFailure type: {}\nNotes: {}\n\n",
        review.get("agent_id").and_then(|v| v.as_str()).unwrap_or("unknown"),
        review.get("session_id").and_then(|v| v.as_str()).unwrap_or("unknown"),
        review.get("model").and_then(|v| v.as_str()).unwrap_or("unknown"),
        review.get("overall_label").and_then(|v| v.as_str()).unwrap_or("unlabeled"),
        review.get("failure_type").and_then(|v| v.as_str()).unwrap_or("none"),
        review.get("notes").and_then(|v| v.as_str()).unwrap_or("")
    ));

    for (idx, step) in steps.iter().enumerate() {
        let kind = step.get("kind").and_then(|v| v.as_str()).unwrap_or("event");
        text.push_str(&format!("Step {}: {}\n", idx + 1, kind));
        match kind {
            "tool" => {
                text.push_str(&format!(
                    "tool_call: {}\n",
                    step.get("tool_call").cloned().unwrap_or(json!(null))
                ));
                text.push_str(&format!(
                    "tool_result: {}\n",
                    step.get("tool_result").cloned().unwrap_or(json!(null))
                ));
            }
            _ => {
                text.push_str(&format!(
                    "request: {}\n",
                    step.get("request").cloned().unwrap_or(json!(null))
                ));
                text.push_str(&format!(
                    "response: {}\n",
                    step.get("response").cloned().unwrap_or(json!(null))
                ));
            }
        }
        text.push('\n');
    }

    text
}

fn build_assist_summary(steps: &[Value]) -> String {
    let mut out = String::new();

    for (idx, step) in steps.iter().enumerate() {
        let kind = step.get("kind").and_then(|v| v.as_str()).unwrap_or("event");
        let event_id = primary_step_event_id(step).unwrap_or_default();
        out.push_str(&format!("Step {} [{}] event_id={}\n", idx + 1, kind, event_id));

        if kind == "tool" {
            let call = step.get("tool_call").cloned().unwrap_or(json!(null));
            let result = step.get("tool_result").cloned().unwrap_or(json!(null));
            out.push_str(&format!("tool_call: {}\n", compact_json(&call, 900)));
            out.push_str(&format!("tool_result: {}\n\n", compact_json(&result, 1200)));
            continue;
        }

        let request = step.get("request").cloned().unwrap_or(json!(null));
        let response = step.get("response").cloned().unwrap_or(json!(null));
        out.push_str(&format!("request: {}\n", summarize_request(&request)));
        out.push_str(&format!("response: {}\n\n", summarize_response(&response)));
    }

    out
}

fn summarize_request(request: &Value) -> String {
    let messages = request
        .get("messages")
        .and_then(|v| v.as_array())
        .map(|messages| {
            messages
                .iter()
                .filter_map(|message| {
                    let role = message.get("role").and_then(|v| v.as_str()).unwrap_or("unknown");
                    if role == "system" {
                        return None;
                    }
                    let content = message
                        .get("content")
                        .map(|v| compact_json(v, 500))
                        .unwrap_or_else(|| "null".to_string());
                    Some(format!("{}: {}", role, content))
                })
                .collect::<Vec<_>>()
                .join(" | ")
        })
        .unwrap_or_else(|| compact_json(request, 900));
    truncate(&messages, 1200)
}

fn summarize_response(response: &Value) -> String {
    let message = response
        .get("choices")
        .and_then(|v| v.get(0))
        .and_then(|v| v.get("message"))
        .cloned()
        .unwrap_or_else(|| json!(null));
    truncate(&compact_json(&message, 1400), 1400)
}

fn compact_json(value: &Value, max_chars: usize) -> String {
    truncate(&value.to_string(), max_chars)
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut out: String = value.chars().take(max_chars).collect();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

fn normalize_label(label: &str) -> String {
    match label {
        "good" | "bad" | "needs_review" => label.to_string(),
        _ => "needs_review".to_string(),
    }
}

fn normalize_failure_type(value: Option<&str>) -> Option<String> {
    match value {
        Some("bad_answer" | "bad_tool_use" | "hallucination" | "inefficient" | "unsafe" | "other") => {
            value.map(|v| v.to_string())
        }
        _ => None,
    }
}

fn primary_step_event_id(step: &Value) -> Option<String> {
    let ids = step.get("event_ids")?;
    ids.get("response")
        .or_else(|| ids.get("tool_result_sent_in"))
        .or_else(|| ids.get("tool_requested_by"))
        .or_else(|| ids.get("request"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn fallback_failure_step_index(label: &str, failure_type: Option<&str>, steps: &[Value]) -> Option<usize> {
    if label == "good" {
        return None;
    }

    match failure_type {
        Some("bad_tool_use" | "inefficient") => steps
            .iter()
            .position(|step| {
                if step.get("kind").and_then(|v| v.as_str()) != Some("model") {
                    return false;
                }
                step.get("response")
                    .and_then(|response| response.get("choices"))
                    .and_then(|choices| choices.get(0))
                    .and_then(|choice| choice.get("message"))
                    .and_then(|message| message.get("tool_calls"))
                    .and_then(|calls| calls.as_array())
                    .map(|calls| !calls.is_empty())
                    .unwrap_or(false)
            })
            .map(|idx| idx + 1),
        Some("bad_answer" | "hallucination" | "unsafe") => steps
            .iter()
            .rposition(|step| step.get("kind").and_then(|v| v.as_str()) == Some("model"))
            .map(|idx| idx + 1),
        _ => None,
    }
}

fn normalize_prime_tool_review(assist: &mut Value, steps: &[Value]) {
    let Some(number) = extract_prime_question_number(steps) else {
        return;
    };
    let Some(tool_step_index) = first_model_tool_request_index(steps) else {
        return;
    };
    let Some(final_answer) = final_model_answer(steps) else {
        return;
    };

    let expected_prime = is_prime(number);
    let final_answer_correct = prime_answer_matches(&final_answer, number, expected_prime);

    if !final_answer_correct || !uses_divide_number_tool(steps) {
        return;
    }

    let failure_type = assist.get("failure_type").and_then(|v| v.as_str());
    let critique = assist
        .get("critique")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let incorrect_signal = assist
        .get("quality_signals")
        .and_then(|v| v.as_array())
        .map(|signals| {
            signals.iter().any(|signal| {
                signal
                    .as_str()
                    .map(|s| s.contains("incorrect_final_answer") || s.contains("bad_answer"))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);

    if failure_type == Some("bad_answer") || incorrect_signal || critique.contains("misleading") {
        assist["suggested_label"] = json!("needs_review");
        assist["failure_type"] = json!("bad_tool_use");
        assist["failure_step_index"] = json!(tool_step_index);
        assist["critique"] = json!("The final answer is correct, but the model made an unnecessary tool-use decision for a simple prime check. The failure should be marked on the model turn that requested the tool.");
        assist["quality_signals"] = json!(["correct_final_answer", "unnecessary_tool_use", "tool_use_decision"]);
    }
}

fn extract_prime_question_number(steps: &[Value]) -> Option<u64> {
    let prompt = steps
        .iter()
        .find(|step| step.get("kind").and_then(|v| v.as_str()) == Some("model"))
        .and_then(|step| step.get("request"))
        .and_then(|request| request.get("messages"))
        .and_then(|messages| messages.as_array())
        .and_then(|messages| {
            messages.iter().find_map(|message| {
                if message.get("role").and_then(|v| v.as_str()) != Some("user") {
                    return None;
                }
                message.get("content").and_then(|v| v.as_str())
            })
        })?;

    if !prompt.to_ascii_lowercase().contains("prime") {
        return None;
    }

    prompt
        .split(|ch: char| !ch.is_ascii_digit())
        .find(|part| !part.is_empty())
        .and_then(|part| part.parse::<u64>().ok())
}

fn first_model_tool_request_index(steps: &[Value]) -> Option<usize> {
    steps
        .iter()
        .position(|step| {
            if step.get("kind").and_then(|v| v.as_str()) != Some("model") {
                return false;
            }
            step.get("response")
                .and_then(|response| response.get("choices"))
                .and_then(|choices| choices.get(0))
                .and_then(|choice| choice.get("message"))
                .and_then(|message| message.get("tool_calls"))
                .and_then(|calls| calls.as_array())
                .map(|calls| !calls.is_empty())
                .unwrap_or(false)
        })
        .map(|idx| idx + 1)
}

fn final_model_answer(steps: &[Value]) -> Option<String> {
    steps
        .iter()
        .rev()
        .find(|step| step.get("kind").and_then(|v| v.as_str()) == Some("model"))
        .and_then(|step| step.get("response"))
        .and_then(|response| response.get("choices"))
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .map(|s| s.to_string())
}

fn uses_divide_number_tool(steps: &[Value]) -> bool {
    steps.iter().any(|step| {
        step.get("tool_call")
            .and_then(|call| call.get("function"))
            .and_then(|function| function.get("name"))
            .and_then(|name| name.as_str())
            == Some("divide_number")
    })
}

fn is_prime(number: u64) -> bool {
    if number < 2 {
        return false;
    }
    let mut divisor = 2;
    while divisor * divisor <= number {
        if number % divisor == 0 {
            return false;
        }
        divisor += 1;
    }
    true
}

fn prime_answer_matches(answer: &str, number: u64, expected_prime: bool) -> bool {
    if let Ok(value) = serde_json::from_str::<Value>(answer) {
        let answer_number = value.get("number").and_then(|v| v.as_u64());
        let answer_prime = value.get("is_prime").and_then(|v| v.as_bool());
        if answer_number == Some(number) && answer_prime == Some(expected_prime) {
            return true;
        }
    }

    let answer = answer.to_ascii_lowercase();
    let says_prime = answer.contains("is prime") || answer.contains("prime number");
    let says_not_prime = answer.contains("not prime") || answer.contains("isn't prime") || answer.contains("is not a prime");
    if expected_prime {
        says_prime && !says_not_prime
    } else {
        says_not_prime
    }
}

fn parse_assist_response(content: &str) -> Value {
    serde_json::from_str::<Value>(content).unwrap_or_else(|_| {
        json!({
            "suggested_label": "needs_review",
            "confidence": 0.0,
            "failure_type": "other",
            "failure_event_id": null,
            "critique": truncate(content, 500),
            "quality_signals": ["unstructured_assistant_response"]
        })
    })
}

fn coerce_quality_signals(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(|s| s.to_string()))
            .collect(),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(key, value)| {
                if let Some(text) = value.as_str() {
                    format!("{}: {}", key, text)
                } else {
                    key.to_string()
                }
            })
            .collect(),
        Some(Value::String(text)) => vec![text.to_string()],
        _ => Vec::new(),
    }
}

fn build_logical_steps(events: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut steps = Vec::new();

    for (idx, event) in events.iter().enumerate() {
        if event.get("direction").and_then(|v| v.as_str()) != Some("outbound") {
            continue;
        }

        let response = events
            .get(idx + 1)
            .filter(|next| next.get("direction").and_then(|v| v.as_str()) == Some("inbound"));

        steps.push(json!({
            "kind": "model",
            "event_ids": {
                "request": event.get("id").cloned().unwrap_or(json!(null)),
                "response": response.and_then(|v| v.get("id")).cloned().unwrap_or(json!(null))
            },
            "timestamp": event.get("timestamp").cloned().unwrap_or(json!(null)),
            "model": event.get("model").cloned().unwrap_or(json!(null)),
            "request": event.get("payload").and_then(|v| v.get("request")).cloned().unwrap_or(json!(null)),
            "response": response
                .and_then(|v| v.get("payload"))
                .and_then(|v| v.get("response"))
                .cloned()
                .unwrap_or(json!(null))
        }));

        let Some(response_event) = response else {
            continue;
        };

        let tool_results = events
            .get(idx + 2)
            .and_then(|next| next.get("payload"))
            .and_then(|payload| payload.get("request"))
            .and_then(|request| request.get("messages"))
            .and_then(|messages| messages.as_array())
            .map(|messages| {
                messages
                    .iter()
                    .filter(|message| message.get("role").and_then(|v| v.as_str()) == Some("tool"))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        for tool_call in response_event
            .get("payload")
            .and_then(|v| v.get("response"))
            .and_then(|response| response.get("choices"))
            .and_then(|choices| choices.get(0))
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("tool_calls"))
            .and_then(|calls| calls.as_array())
            .into_iter()
            .flatten()
        {
            let call_id = tool_call.get("id").and_then(|v| v.as_str());
            let call_name = tool_call
                .get("function")
                .and_then(|v| v.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("tool");

            let tool_result = tool_results.iter().find(|result| {
                result.get("tool_call_id").and_then(|v| v.as_str()) == call_id
                    || result.get("name").and_then(|v| v.as_str()) == Some(call_name)
            });

            steps.push(json!({
                "kind": "tool",
                "event_ids": {
                    "tool_requested_by": response_event.get("id").cloned().unwrap_or(json!(null)),
                    "tool_result_sent_in": events.get(idx + 2).and_then(|v| v.get("id")).cloned().unwrap_or(json!(null))
                },
                "timestamp": response_event.get("timestamp").cloned().unwrap_or(json!(null)),
                "tool_call": tool_call,
                "tool_result": tool_result.cloned().unwrap_or(json!(null))
            }));
        }
    }

    steps
}
