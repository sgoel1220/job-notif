use super::{
    locations::merge_locations,
    normalization::{canonical_salary_interval, normalize_employment},
    JobPosting,
};
use crate::errors::ApiError;
use sqlx::PgPool;

pub(super) fn location_for_storage(job: &JobPosting) -> Option<String> {
    match serde_json::from_str(&job.details_json) {
        Ok(details) => merge_locations(job.location.clone(), &details),
        // Raw metadata must never erase an already normalized location.
        Err(_) => job.location.clone(),
    }
}

/// Reconcile one company's complete snapshot for one exact ATS source.
///
/// The exact source equality avoids prefix collisions such as `lever` matching
/// `lever-v2`. Validation runs before the transaction so a malformed snapshot
/// cannot deactivate existing rows for the source.
pub(crate) async fn replace_source_snapshot(
    db: &PgPool,
    company: &str,
    source: &str,
    jobs: &[JobPosting],
) -> Result<(), ApiError> {
    if jobs
        .iter()
        .any(|job| job.company != company || job.source != source)
    {
        return Err(ApiError::bad_request(
            "Snapshot contains jobs outside the requested company/source.",
        ));
    }

    let mut tx = db.begin().await.map_err(ApiError::database)?;
    sqlx::query("UPDATE job_postings SET is_active = FALSE WHERE company = $1 AND source = $2")
        .bind(company)
        .bind(source)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    for job in jobs {
        sqlx::query(
            "INSERT INTO job_postings (
                source, source_job_id, title, company, location, workplace_type, employment_type,
                department, team, description, description_text, posted_at, salary_min, salary_max,
                salary_currency, salary_interval, url, details_json, is_active
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, TRUE)
             ON CONFLICT(company, source, source_job_id) DO UPDATE SET
                title = excluded.title, location = excluded.location,
                workplace_type = excluded.workplace_type, employment_type = excluded.employment_type,
                department = excluded.department, team = excluded.team,
                description = excluded.description, description_text = excluded.description_text,
                posted_at = excluded.posted_at, salary_min = excluded.salary_min,
                salary_max = excluded.salary_max, salary_currency = excluded.salary_currency,
                salary_interval = excluded.salary_interval,
                url = excluded.url, details_json = excluded.details_json,
                last_seen_at = CURRENT_TIMESTAMP::text, is_active = TRUE"
        )
        .bind(&job.source).bind(&job.source_job_id).bind(&job.title).bind(&job.company)
        .bind(location_for_storage(job)).bind(&job.workplace_type).bind(normalize_employment(job.employment_type.clone()))
        .bind(&job.department).bind(&job.team).bind(&job.description).bind(&job.description_text)
        .bind(&job.posted_at).bind(job.salary_min).bind(job.salary_max).bind(&job.salary_currency)
        .bind(job.salary_interval.as_deref().and_then(canonical_salary_interval)).bind(&job.url).bind(&job.details_json)
        .execute(&mut *tx).await.map_err(ApiError::database)?;
    }
    tx.commit().await.map_err(ApiError::database)
}
