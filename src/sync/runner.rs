use crate::{ats, company_registry, errors, job_postings, remote, signals, state::AppState};
use axum::http::StatusCode;
use sqlx::PgPool;
use std::str::FromStr;

pub(super) async fn retire_aggregate_fallbacks(
    db: &PgPool,
    successfully_replaced: &[String],
) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;
    for company in successfully_replaced {
        sqlx::query("UPDATE job_postings SET is_active = FALSE WHERE company = $1 AND source LIKE 'open-jobs-data/%' AND is_active = TRUE")
            .bind(company)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await
}

pub(super) fn require_ats_fetch_success<T>(
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

pub(super) async fn reconcile_company_snapshot(
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

pub(super) async fn sync_all_locked(state: &AppState) -> Result<(), errors::ApiError> {
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

/// Run reconciliation and persist its completion marker while the caller still
/// holds the process-wide sync lock, so competing runs cannot reorder timestamps.
pub(super) async fn sync_and_mark_success_locked(state: &AppState) -> Result<(), errors::ApiError> {
    sync_then_mark_success(state, sync_all_locked(state)).await
}

pub(super) async fn sync_then_mark_success<F>(
    state: &AppState,
    operation: F,
) -> Result<(), errors::ApiError>
where
    F: std::future::Future<Output = Result<(), errors::ApiError>>,
{
    operation.await?;
    sqlx::query("UPDATE sync_state SET last_successful_sync = CURRENT_TIMESTAMP WHERE id = 1")
        .execute(&state.db)
        .await
        .map_err(errors::ApiError::database)?;
    Ok(())
}
