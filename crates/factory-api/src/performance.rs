//! Read-only, semantic performance endpoints backed by the evaluation
//! engine. They expose measured agent performance from local workflow
//! history; there is no arbitrary SQL surface and nothing here mutates
//! execution state.

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use factory_eval::{
    evaluate, evaluate_agent, AgentPerformanceDetail, AgentPerformanceSummary, PerformanceFacets,
    PerformanceQuery, PerformanceWindow,
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

/// `GET /api/performance/agents/:agent` — full detail with breakdowns,
/// trends, and reasons. 404 when the agent has no attributed task history.
pub(crate) async fn get_agent_performance(
    State(state): State<SharedState>,
    UrlPath(agent): UrlPath<String>,
    Query(params): Query<PerformanceQueryParams>,
) -> Result<Json<AgentPerformanceDetail>, ApiError> {
    let query = parse_query(params)?;
    let db = state.db.lock().expect("db mutex poisoned");
    evaluate_agent(&db, &agent, &query, Utc::now())?
        .map(Json)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                format!("no performance data for agent '{agent}'"),
            )
        })
}
