mod ats;
mod background_jobs;
mod company_registry;
mod errors;
#[allow(dead_code)]
mod gitlab;
mod greetings;
#[allow(dead_code)]
mod jeeves;
mod job_postings;
#[allow(dead_code)]
mod linear;
mod listings;
mod models;
#[allow(dead_code)]
mod moonpay;
#[allow(dead_code)]
mod open_jobs_data;
mod remote;
mod signals;
#[allow(dead_code)]
mod supabase;
mod ui;

use std::{str::FromStr, sync::Arc};

use axum::{http::StatusCode, routing::get, Router};
use sqlx::{postgres::PgPoolOptions, PgPool};

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) db: PgPool,
    sync_lock: Arc<tokio::sync::Mutex<()>>,
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

    sqlx::query("UPDATE background_jobs SET status = 'queued' WHERE status = 'running'")
        .execute(&db)
        .await?;
    tokio::spawn(background_jobs::run_worker(db.clone()));

    let state = AppState {
        db,
        sync_lock: Arc::new(tokio::sync::Mutex::new(())),
    };
    let app = Router::new()
        .route("/", get(ui::home))
        .route(
            "/api/greetings",
            get(greetings::list).post(greetings::create),
        )
        .route("/api/listings", get(listings::list))
        .route("/api/job-postings", get(job_postings::list))
        .route("/api/sync", axum::routing::post(sync_all))
        .route(
            "/api/jobs",
            get(background_jobs::list).post(background_jobs::enqueue),
        )
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

async fn sync_all(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<axum::http::StatusCode, errors::ApiError> {
    sync_all_inner(&state).await?;
    sqlx::query("UPDATE sync_state SET last_successful_sync = CURRENT_TIMESTAMP WHERE id = 1")
        .execute(&state.db)
        .await
        .map_err(errors::ApiError::database)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn sync_all_inner(state: &AppState) -> Result<(), errors::ApiError> {
    use futures_util::stream::{self, StreamExt};

    // Prevent manual and scheduled syncs from reconciling the same sources concurrently.
    let _guard = state.sync_lock.lock().await;

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
    let mut failures: Vec<String> = stream::iter(companies)
        .map(|company| {
            let state = state.clone();
            let client = client.clone();
            async move { sync_one_company(&state, &client, &company).await }
        })
        .buffer_unordered(MAX_PARALLEL_SOURCES)
        .filter_map(|result| async move { result.err() })
        .collect()
        .await;
    failures.sort();

    // The aggregate snapshot is retired. Keep its rows for history, but never
    // expose them as active listings once direct feeds are in use.
    sqlx::query("UPDATE job_postings SET is_active = FALSE WHERE source LIKE 'open-jobs-data/%' AND is_active = TRUE")
        .execute(&state.db)
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
        "remote-public" => remote::sync(State(state.clone()))
            .await
            .map(|_| ())
            .map_err(|error| format!("{} {}", error.0, error.1)),
        "atom-feed" => signals::sync(State(state.clone()))
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
            let fetched_jobs = match fetch_ats_with_retry(client, &source).await {
                Ok(jobs) => jobs,
                // A permanent client error (e.g. Workday 422 for a retired or
                // misconfigured board) cannot be fixed by retrying the whole
                // sync. Isolate it to this company so healthy feeds can complete.
                Err(error @ ats::AtsFetchError::Http { status, .. })
                    if status.is_client_error() && status.as_u16() != 429 =>
                {
                    eprintln!("Skipping {name} for this sync after permanent ATS error: {error}");
                    return Ok(());
                }
                Err(error) => return Err(error.to_string()),
            };
            let postings: Vec<_> = fetched_jobs.into_iter().map(|job| job.posting).collect();
            if postings
                .iter()
                .any(|job| job.company != name || job.source != source_key)
            {
                return Err(format!(
                    "ATS returned jobs outside expected company/source {name}/{source_key}"
                ));
            }
            job_postings::replace_source_snapshot(&state.db, name, &source_key, &postings)
                .await
                .map_err(|error| format!("{} {}", error.0, error.1))?;

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
