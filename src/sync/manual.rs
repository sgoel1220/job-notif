use super::{runner::sync_and_mark_success_locked, scheduler::TOTAL_SYNC_TIMEOUT};
use crate::{errors, state::AppState};
use axum::http::{HeaderMap, StatusCode};
use std::time::{Duration, Instant};

const MANUAL_SYNC_RATE_LIMIT: Duration = Duration::from_secs(30);

pub(crate) async fn sync_all(
    axum::extract::State(state): axum::extract::State<AppState>,
    headers: HeaderMap,
) -> Result<axum::http::StatusCode, errors::ApiError> {
    let expected = std::env::var("SYNC_BEARER_TOKEN").map_err(|_| {
        eprintln!("SYNC_BEARER_TOKEN is not configured; manual sync is disabled");
        errors::ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Manual sync is not configured.".into(),
        )
    })?;
    if expected.trim().is_empty() {
        eprintln!("SYNC_BEARER_TOKEN is empty; manual sync is disabled");
        return Err(errors::ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Manual sync is not configured.".into(),
        ));
    }
    let supplied = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if !supplied.is_some_and(|value| constant_time_eq(value.as_bytes(), expected.as_bytes())) {
        return Err(errors::ApiError(
            StatusCode::UNAUTHORIZED,
            "Unauthorized.".into(),
        ));
    }

    let _guard = state.sync_lock.try_lock().map_err(|_| {
        errors::ApiError(
            StatusCode::CONFLICT,
            "A synchronization is already running.".into(),
        )
    })?;
    {
        let mut last = state.manual_sync_last_request.lock().map_err(|_| {
            errors::ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "Manual sync is unavailable.".into(),
            )
        })?;
        if last.is_some_and(|time| time.elapsed() < MANUAL_SYNC_RATE_LIMIT) {
            return Err(errors::ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "Manual sync is rate limited.".into(),
            ));
        }
        *last = Some(Instant::now());
    }

    tokio::time::timeout(TOTAL_SYNC_TIMEOUT, sync_and_mark_success_locked(&state))
        .await
        .map_err(|_| {
            errors::ApiError(
                StatusCode::GATEWAY_TIMEOUT,
                "Synchronization exceeded its time limit.".into(),
            )
        })??;
    Ok(StatusCode::NO_CONTENT)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    bool::from(left.ct_eq(right))
}
