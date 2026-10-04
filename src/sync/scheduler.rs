use super::runner::sync_and_mark_success_locked;
use crate::{errors, state::AppState};
use axum::http::StatusCode;
use std::time::Duration;

pub(crate) const TOTAL_SYNC_TIMEOUT: Duration = Duration::from_secs(45 * 60);
pub(crate) async fn run_daily_sync(state: AppState) {
    use tokio::time::{interval, Duration};

    let mut poll = interval(Duration::from_secs(5 * 60));
    loop {
        poll.tick().await;
        let due = sqlx::query_scalar::<_, bool>(
            "SELECT last_successful_sync IS NULL
                 OR last_successful_sync <= CURRENT_TIMESTAMP - INTERVAL '24 hours'
             FROM sync_state WHERE id = 1",
        )
        .fetch_one(&state.db)
        .await;
        match due {
            Ok(true) => match sync_all_inner(&state).await {
                Ok(()) => println!("All job sources synced successfully"),
                Err(error) => eprintln!(
                    "Full job sync failed; it will be retried: {} {}",
                    error.0, error.1
                ),
            },
            Ok(_) => {}
            Err(error) => eprintln!("Could not read last sync time: {error}"),
        }
    }
}

pub(super) async fn sync_all_inner(state: &AppState) -> Result<(), errors::ApiError> {
    with_sync_lock_timeout(
        &state.sync_lock,
        TOTAL_SYNC_TIMEOUT,
        sync_and_mark_success_locked(state),
    )
    .await
}

pub(super) async fn with_sync_lock_timeout<F>(
    lock: &tokio::sync::Mutex<()>,
    timeout: Duration,
    operation: F,
) -> Result<(), errors::ApiError>
where
    F: std::future::Future<Output = Result<(), errors::ApiError>>,
{
    let _guard = lock.lock().await;
    tokio::time::timeout(timeout, operation)
        .await
        .map_err(|_| {
            errors::ApiError(
                StatusCode::GATEWAY_TIMEOUT,
                "Synchronization exceeded its time limit.".into(),
            )
        })?
}
