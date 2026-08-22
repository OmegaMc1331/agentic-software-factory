//! Read-only, semantic performance endpoints backed by the evaluation
//! engine. They expose measured agent performance from local workflow
//! history; there is no arbitrary SQL surface and nothing here mutates
//! execution state.

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use factory_eval::{
    evaluate, evaluate_agent, AgentMetrics, AgentPerformanceDetail, AgentPerformanceSummary,
    PerformanceFacets, PerformanceQuery, PerformanceWindow, MIN_RELIABLE_RATE_SAMPLES,
};
use factory_types::TaskOperation;
use serde::Deserialize;

use crate::app::{ApiError, SharedState};

#[derive(Debug, Default, Deserialize)]
pub(crate) struct PerformanceQueryParams {
    window: Option<String>,
    role: Option<String>,
    operation: Option<String>,
    language: Option<String>,
}

fn parse_query(params: PerformanceQueryParams) -> Result<PerformanceQuery, ApiError> {
    let window = match params.window.as_deref() {
        None | Some("") => PerformanceWindow::AllTime,
        Some(raw) => PerformanceWindow::parse(raw).ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("unknown window '{raw}' (expected all, 30d, or 7d)"),
            )
        })?,
    };
    let operation = match params.operation.as_deref() {
        None | Some("") => None,
        Some(raw) => Some(raw.parse::<TaskOperation>().map_err(|_| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("unknown operation '{raw}'"),
            )
        })?),
    };
    Ok(PerformanceQuery {
        window,
        agent: None,
        role: params.role.filter(|role| !role.is_empty()),
        operation,
        language: params.language.filter(|language| !language.is_empty()),
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PerformanceOverviewResponse {
    window: PerformanceWindow,
    agents: Vec<AgentPerformanceSummary>,
    facets: PerformanceFacets,
}

/// `GET /api/performance/agents` — one compact summary per agent, with the
/// filter values observed in the evaluated window.
pub(crate) async fn list_agent_performance(
    State(state): State<SharedState>,
    Query(params): Query<PerformanceQueryParams>,
) -> Result<Json<PerformanceOverviewResponse>, ApiError> {
    let query = parse_query(params)?;
    let db = state.db.lock().expect("db mutex poisoned");
    let report = evaluate(&db, &query, Utc::now())?;
    Ok(Json(PerformanceOverviewResponse {
        window: report.window,
        agents: report.agents,
        facets: report.facets,
    }))
}

/// Whether (and how) an agent's measured performance currently feeds the
/// routing scheduler. Displayed in the Performance view next to the metrics.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoutingUsage {
    /// The configured routing mode (`round_robin`, `performance`, `manual`).
    mode: String,
    /// Whether these metrics can influence dispatch right now.
    used_for_routing: bool,
    /// Why: reliable slice or insufficient sample size, with the threshold.
    note: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentPerformanceDetailResponse {
    #[serde(flatten)]
    detail: AgentPerformanceDetail,
    routing: RoutingUsage,
}

fn routing_usage(state: &SharedState, metrics: &AgentMetrics) -> RoutingUsage {
    let mode = factory_core::Config::load(&state.root)
        .map(|config| config.routing.mode.as_str().to_string())
        .unwrap_or_else(|_| "round_robin".to_string());
    let used = mode == "performance";
    let qualifying = metrics.qualifying_tasks;
    let reliable = metrics.eventual_approval.reliable;
    let note = if reliable {
        format!("Reliable quality slice (n={qualifying}) — eligible for performance routing.")
    } else {
        format!(
            "Insufficient samples (n={qualifying} of {MIN_RELIABLE_RATE_SAMPLES}) — excluded \
             from performance ranking until reliable."
        )
    };
    RoutingUsage {
        mode,
        used_for_routing: used,
        note,
    }
}

/// `GET /api/performance/agents/:agent` — full detail with breakdowns,
/// trends, and reasons, plus whether the metrics currently feed routing.
/// 404 when the agent has no attributed task history.
pub(crate) async fn get_agent_performance(
    State(state): State<SharedState>,
    UrlPath(agent): UrlPath<String>,
    Query(params): Query<PerformanceQueryParams>,
) -> Result<Json<AgentPerformanceDetailResponse>, ApiError> {
    let query = parse_query(params)?;
    let db = state.db.lock().expect("db mutex poisoned");
    let detail = evaluate_agent(&db, &agent, &query, Utc::now())?.ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no performance data for agent '{agent}'"),
        )
    })?;
    let routing = routing_usage(&state, &detail.summary.metrics);
    Ok(Json(AgentPerformanceDetailResponse { detail, routing }))
}
