use super::manual::sync_all;
use super::runner::*;
use super::scheduler::*;
use crate::{ats, job_postings, state::AppState};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
};
use sqlx::PgPool;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
#[cfg(test)]
pub(crate) async fn test_database() -> PgPool {
    use sqlx::postgres::PgPoolOptions;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_SCHEMA: AtomicUsize = AtomicUsize::new(0);
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set to run database tests");
    let schema = format!(
        "job_notif_test_{}_{}",
        std::process::id(),
        NEXT_SCHEMA.fetch_add(1, Ordering::Relaxed)
    );
    let db = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .unwrap();
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&db)
        .await
        .unwrap();
    sqlx::query(&format!("SET search_path TO {schema}"))
        .execute(&db)
        .await
        .unwrap();
    sqlx::migrate!().run(&db).await.unwrap();
    db
}

#[cfg(test)]
mod sync_safety_tests {
    use super::*;
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn state(db: PgPool) -> AppState {
        AppState {
            db,
            sync_lock: Arc::new(tokio::sync::Mutex::new(())),
            manual_sync_last_request: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    async fn insert_posting(db: &PgPool, company: &str, source: &str, id: &str) {
        sqlx::query("INSERT INTO job_postings (source, source_job_id, title, company, url) VALUES ($1, $2, 'Engineer', $3, 'https://example.test/job')")
            .bind(source).bind(id).bind(company).execute(db).await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn manual_route_fails_closed_before_mutation_for_missing_or_invalid_bearer() {
        let _env_lock = ENV_LOCK.lock().await;
        let old = std::env::var_os("SYNC_BEARER_TOKEN");
        let state = state(test_database().await);

        std::env::remove_var("SYNC_BEARER_TOKEN");
        let missing = sync_all(State(state.clone()), HeaderMap::new())
            .await
            .unwrap_err();
        assert_eq!(missing.0, StatusCode::SERVICE_UNAVAILABLE);

        std::env::set_var("SYNC_BEARER_TOKEN", "test-secret-only");
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer wrong".parse().unwrap(),
        );
        let invalid = sync_all(State(state.clone()), headers).await.unwrap_err();
        assert_eq!(invalid.0, StatusCode::UNAUTHORIZED);
        let last_sync_is_null: bool =
            sqlx::query_scalar("SELECT last_successful_sync IS NULL FROM sync_state WHERE id = 1")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert!(last_sync_is_null);
        if let Some(old) = old {
            std::env::set_var("SYNC_BEARER_TOKEN", old);
        } else {
            std::env::remove_var("SYNC_BEARER_TOKEN");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn manual_route_rejects_busy_and_cooldown_without_queueing_syncs() {
        let _env_lock = ENV_LOCK.lock().await;
        let old = std::env::var_os("SYNC_BEARER_TOKEN");
        std::env::set_var("SYNC_BEARER_TOKEN", "test-secret-only");
        let state = state(test_database().await);
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer test-secret-only".parse().unwrap(),
        );

        let guard = state.sync_lock.lock().await;
        let busy = sync_all(State(state.clone()), headers.clone())
            .await
            .unwrap_err();
        assert_eq!(busy.0, StatusCode::CONFLICT);
        drop(guard);

        *state.manual_sync_last_request.lock().unwrap() = Some(Instant::now());
        let limited = sync_all(State(state.clone()), headers).await.unwrap_err();
        assert_eq!(limited.0, StatusCode::TOO_MANY_REQUESTS);
        if let Some(old) = old {
            std::env::set_var("SYNC_BEARER_TOKEN", old);
        } else {
            std::env::remove_var("SYNC_BEARER_TOKEN");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn completion_timestamp_is_persisted_before_sync_lock_is_released() {
        let db = test_database().await;
        let state = state(db.clone());
        let guard = state.sync_lock.lock().await;
        sync_then_mark_success(&state, async { Ok(()) })
            .await
            .unwrap();
        assert!(state.sync_lock.try_lock().is_err());
        let marked: bool = sqlx::query_scalar(
            "SELECT last_successful_sync IS NOT NULL FROM sync_state WHERE id = 1",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert!(marked);
        drop(guard);
        assert!(state.sync_lock.try_lock().is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn failed_or_timed_out_sync_keeps_previous_completion_timestamp() {
        let state = state(test_database().await);
        sqlx::query(
            "UPDATE sync_state SET last_successful_sync = '2000-01-01T00:00:00Z' WHERE id = 1",
        )
        .execute(&state.db)
        .await
        .unwrap();
        let failed = with_sync_lock_timeout(
            &state.sync_lock,
            Duration::from_secs(1),
            sync_then_mark_success(&state, async {
                Err(crate::errors::ApiError(
                    StatusCode::BAD_GATEWAY,
                    "fixture failure".into(),
                ))
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(failed.0, StatusCode::BAD_GATEWAY);
        let timed_out = with_sync_lock_timeout(
            &state.sync_lock,
            Duration::from_millis(10),
            sync_then_mark_success(&state, std::future::pending()),
        )
        .await
        .unwrap_err();
        assert_eq!(timed_out.0, StatusCode::GATEWAY_TIMEOUT);
        assert!(state.sync_lock.try_lock().is_ok());
        let unchanged: bool = sqlx::query_scalar(
            "SELECT last_successful_sync = '2000-01-01T00:00:00Z' FROM sync_state WHERE id = 1",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert!(unchanged);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn total_timeout_drops_operation_and_releases_sync_lock() {
        let lock = tokio::sync::Mutex::new(());
        let result =
            with_sync_lock_timeout(&lock, Duration::from_millis(10), std::future::pending()).await;
        assert_eq!(result.unwrap_err().0, StatusCode::GATEWAY_TIMEOUT);
        assert!(lock.try_lock().is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn permanent_403_stays_failed_and_only_successfully_replaced_company_fallback_retires() {
        let db = test_database().await;
        insert_posting(&db, "Failing", "open-jobs-data/greenhouse", "fallback-fail").await;
        insert_posting(&db, "Succeeded", "open-jobs-data/greenhouse", "fallback-ok").await;
        insert_posting(
            &db,
            "Neighbor",
            "open-jobs-data/greenhouse-v2",
            "fallback-neighbor",
        )
        .await;
        insert_posting(
            &db,
            "Unresolved",
            "open-jobs-data/greenhouse",
            "fallback-unresolved",
        )
        .await;

        let failed_fetch = require_ats_fetch_success::<Vec<ats::FetchedAtsJob>>(
            "Failing",
            Err(ats::AtsFetchError::Http {
                provider: ats::AtsProvider::Greenhouse,
                company: "Failing".into(),
                url: "https://example.test/board/jobs".into(),
                status: StatusCode::FORBIDDEN,
            }),
        );
        assert!(failed_fetch.as_ref().unwrap_err().contains("HTTP 403"));
        assert!(reconcile_company_snapshot(
            &db,
            "Failing",
            "greenhouse/failing",
            Err(failed_fetch.unwrap_err())
        )
        .await
        .is_err());

        let posting = job_postings::JobPosting::basic(
            "greenhouse/succeeded",
            "direct",
            "Engineer".into(),
            "Succeeded",
            None,
            None,
            "https://example.test/direct".into(),
            serde_json::json!({}),
        );
        reconcile_company_snapshot(&db, "Succeeded", "greenhouse/succeeded", Ok(vec![posting]))
            .await
            .unwrap();
        retire_aggregate_fallbacks(&db, &["Succeeded".into()])
            .await
            .unwrap();

        let rows: Vec<(String, bool)> = sqlx::query_as(
            "SELECT source_job_id, is_active FROM job_postings ORDER BY source_job_id",
        )
        .fetch_all(&db)
        .await
        .unwrap();
        assert_eq!(
            rows,
            vec![
                ("direct".into(), true),
                ("fallback-fail".into(), true),
                ("fallback-neighbor".into(), true),
                ("fallback-ok".into(), false),
                ("fallback-unresolved".into(), true)
            ]
        );
    }
}
