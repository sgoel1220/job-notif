mod ats;
mod company_registry;
mod errors;
mod job_postings;
mod listings;
mod remote;
mod signals;
mod ui;

use std::{
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    http::{HeaderMap, StatusCode},
    routing::get,
    Router,
};
use sqlx::{postgres::PgPoolOptions, PgPool};

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) db: PgPool,
    sync_lock: Arc<tokio::sync::Mutex<()>>,
    manual_sync_last_request: Arc<std::sync::Mutex<Option<Instant>>>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set to a PostgreSQL connection string");
    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await?;
    sqlx::migrate!().run(&db).await?;

    let state = AppState {
        db,
        sync_lock: Arc::new(tokio::sync::Mutex::new(())),
        manual_sync_last_request: Arc::new(std::sync::Mutex::new(None)),
    };
    let app = Router::new()
        .route("/", get(ui::home))
        .route("/api/listings", get(listings::list))
        .route("/api/job-postings", get(job_postings::list))
        .route("/api/sync", axum::routing::post(sync_all))
        .with_state(state.clone());

    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "3000".to_owned())
        .parse::<u16>()?;
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tokio::spawn(run_daily_sync(state));
    println!("Job Notifier listening on http://0.0.0.0:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn run_daily_sync(state: AppState) {
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
                Ok(()) => {
                    if let Err(error) = sqlx::query(
                        "UPDATE sync_state SET last_successful_sync = CURRENT_TIMESTAMP WHERE id = 1",
                    )
                    .execute(&state.db)
                    .await
                    {
                        eprintln!("Could not persist completed sync time: {error}");
                    } else {
                        println!("All job sources synced successfully");
                    }
                }
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

const MANUAL_SYNC_RATE_LIMIT: Duration = Duration::from_secs(30);
const TOTAL_SYNC_TIMEOUT: Duration = Duration::from_secs(15 * 60);

async fn sync_all(
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

    tokio::time::timeout(TOTAL_SYNC_TIMEOUT, sync_all_locked(&state))
        .await
        .map_err(|_| {
            errors::ApiError(
                StatusCode::GATEWAY_TIMEOUT,
                "Synchronization exceeded its time limit.".into(),
            )
        })??;
    sqlx::query("UPDATE sync_state SET last_successful_sync = CURRENT_TIMESTAMP WHERE id = 1")
        .execute(&state.db)
        .await
        .map_err(errors::ApiError::database)?;
    Ok(StatusCode::NO_CONTENT)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    bool::from(left.ct_eq(right))
}

async fn sync_all_inner(state: &AppState) -> Result<(), errors::ApiError> {
    with_sync_lock_timeout(&state.sync_lock, TOTAL_SYNC_TIMEOUT, sync_all_locked(state)).await
}

async fn with_sync_lock_timeout<F>(
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

async fn retire_aggregate_fallbacks(
    db: &PgPool,
    successfully_replaced: &[String],
) -> Result<(), sqlx::Error> {
    for company in successfully_replaced {
        sqlx::query("UPDATE job_postings SET is_active = FALSE WHERE company = $1 AND source LIKE 'open-jobs-data/%' AND is_active = TRUE")
            .bind(company)
            .execute(db)
            .await?;
    }
    Ok(())
}

fn require_ats_fetch_success<T>(
    name: &str,
    result: Result<T, ats::AtsFetchError>,
) -> Result<T, String> {
    match result {
        Ok(value) => Ok(value),
        Err(error @ ats::AtsFetchError::Http { status, .. })
            if status.is_client_error() && status.as_u16() != 429 =>
        {
            eprintln!("{name} has a permanent ATS error and remains a failed source: {error}");
            Err(format!("{name}: {error}"))
        }
        Err(error) => Err(error.to_string()),
    }
}

async fn reconcile_company_snapshot(
    db: &PgPool,
    company: &str,
    source: &str,
    postings: Result<Vec<job_postings::JobPosting>, String>,
) -> Result<(), String> {
    let postings = postings?;
    if postings
        .iter()
        .any(|job| job.company != company || job.source != source)
    {
        return Err(format!(
            "ATS returned jobs outside expected company/source {company}/{source}"
        ));
    }
    job_postings::replace_source_snapshot(db, company, source, &postings)
        .await
        .map_err(|error| format!("{} {}", error.0, error.1))?;
    Ok(())
}

async fn sync_all_locked(state: &AppState) -> Result<(), errors::ApiError> {
    use futures_util::stream::{self, StreamExt};

    let registry = company_registry::load_registry().map_err(|error| {
        eprintln!("Company registry JSON could not be loaded: {error}");
        errors::ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Company registry JSON could not be loaded.".into(),
        )
    })?;
    if let Err(validation_errors) = registry.validate() {
        eprintln!("Company registry failed validation: {validation_errors:?}");
        return Err(errors::ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Company registry failed validation: {validation_errors:?}"),
        ));
    }

    let client =
        ats::http_client_with_timeout(std::time::Duration::from_secs(60)).map_err(|error| {
            eprintln!("ATS HTTP client could not be built: {error}");
            errors::ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ATS HTTP client could not be built.".into(),
            )
        })?;

    // Bound outbound load and DB pressure while allowing independent companies to
    // progress concurrently. Each future owns exactly one source reconciliation.
    const MAX_PARALLEL_SOURCES: usize = 8;
    let companies: Vec<_> = registry
        .enabled_companies()
        .filter(|company| company.is_resolved())
        .cloned()
        .collect();
    let outcomes: Vec<_> = stream::iter(companies)
        .map(|company| {
            let state = state.clone();
            let client = client.clone();
            async move {
                let name = company.name.clone();
                (name, sync_one_company(&state, &client, &company).await)
            }
        })
        .buffer_unordered(MAX_PARALLEL_SOURCES)
        .collect()
        .await;
    let mut failures = Vec::new();
    let mut successfully_replaced = Vec::new();
    for (name, outcome) in outcomes {
        match outcome {
            Ok(()) => successfully_replaced.push(name),
            Err(error) => failures.push(error),
        }
    }
    failures.sort();

    // Retire only aggregate rows for companies whose direct source snapshot was
    // successfully reconciled. Failed or unresolved companies retain their fallback.
    retire_aggregate_fallbacks(&state.db, &successfully_replaced)
        .await
        .map_err(errors::ApiError::database)?;

    if failures.is_empty() {
        Ok(())
    } else {
        Err(errors::ApiError(
            StatusCode::BAD_GATEWAY,
            format!("Job aggregator sync failed: {}", failures.join("; ")),
        ))
    }
}

async fn sync_one_company(
    state: &AppState,
    client: &reqwest::Client,
    company: &company_registry::CompanyEntry,
) -> Result<(), String> {
    use axum::extract::State;

    let name = company.name.trim();
    let provider_name = company.provider.as_deref().unwrap_or_default().trim();
    let board = company.board.as_deref().unwrap_or_default().trim();

    match provider_name {
        "remote-public" => remote::sync(State(state.clone()), client)
            .await
            .map(|_| ())
            .map_err(|error| format!("{} {}", error.0, error.1)),
        "atom-feed" => signals::sync(state.clone(), client)
            .await
            .map(|_| ())
            .map_err(|error| format!("{} {}", error.0, error.1)),
        _ => {
            let provider = ats::AtsProvider::from_str(provider_name)
                .map_err(|error| format!("unsupported ATS provider: {error}"))?;
            let workday = if provider == ats::AtsProvider::Workday {
                let parts: Vec<_> = board.split('/').filter(|part| !part.is_empty()).collect();
                if parts.len() != 3 {
                    return Err("Workday board must be tenant/shard/site".into());
                }
                Some(ats::WorkdayConfig {
                    tenant: parts[0].to_owned(),
                    shard: parts[1].to_owned(),
                    site: parts[2].to_owned(),
                })
            } else {
                None
            };
            let source = ats::AtsCompanySource {
                company_id: None,
                company_name: name.to_owned(),
                provider,
                slug: (provider != ats::AtsProvider::Workday).then(|| board.to_owned()),
                workday,
            };
            let identity = source.identity().map_err(|error| error.to_string())?;
            let source_key = identity.source_key();
            // Any fetch error returns before reconciliation, preserving the last good snapshot.
            let fetched_jobs =
                require_ats_fetch_success(name, fetch_ats_with_retry(client, &source).await)?;
            let postings = fetched_jobs.into_iter().map(|job| job.posting).collect();
            reconcile_company_snapshot(&state.db, name, &source_key, Ok(postings)).await?;

            // Retire legacy duplicates only after this source's replacement succeeds.
            sqlx::query(
                "UPDATE job_postings SET is_active = FALSE WHERE company = $1 AND source = $2",
            )
            .bind(name)
            .bind(provider.as_str())
            .execute(&state.db)
            .await
            .map_err(|error| format!("could not retire legacy rows: {error}"))?;
            Ok(())
        }
    }
}

async fn fetch_ats_with_retry(
    client: &reqwest::Client,
    source: &ats::AtsCompanySource,
) -> Result<Vec<ats::FetchedAtsJob>, ats::AtsFetchError> {
    for attempt in 0..2 {
        match ats::fetch_company_jobs(client, source).await {
            Err(error @ ats::AtsFetchError::Request { .. }) if attempt == 0 => {
                eprintln!(
                    "{} transient request failure: {error}; retrying",
                    source.company_name
                );
            }
            Err(error @ ats::AtsFetchError::Http { status, .. })
                if attempt == 0 && (status.as_u16() == 429 || status.is_server_error()) =>
            {
                eprintln!(
                    "{} transient HTTP failure: {error}; retrying",
                    source.company_name
                );
            }
            result => return result,
        }
        tokio::time::sleep(std::time::Duration::from_millis(750)).await;
    }
    unreachable!("second attempt always returns")
}

#[cfg(test)]
mod sync_safety_tests {
    use super::*;
    use axum::extract::State;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn state(db: PgPool) -> AppState {
        AppState {
            db,
            sync_lock: Arc::new(tokio::sync::Mutex::new(())),
            manual_sync_last_request: Arc::new(Mutex::new(None)),
        }
    }

    async fn insert_posting(db: &PgPool, company: &str, source: &str, id: &str) {
        sqlx::query("INSERT INTO job_postings (source, source_job_id, title, company, url) VALUES ($1, $2, 'Engineer', $3, 'https://example.test/job')")
            .bind(source).bind(id).bind(company).execute(db).await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn manual_route_fails_closed_before_mutation_for_missing_or_invalid_bearer() {
        let _env_lock = ENV_LOCK.lock().unwrap();
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
        let _env_lock = ENV_LOCK.lock().unwrap();
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

#[cfg(test)]
pub(crate) async fn test_database() -> PgPool {
    dotenvy::dotenv().ok();
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
