use axum::{extract::State, Json};
use serde::Deserialize;

use crate::{errors::ApiError, AppState};

// Public Lever postings API for Jeeves' official tryjeeves board.
const JOBS_URL: &str = "https://api.lever.co/v0/postings/tryjeeves?mode=json";

#[derive(Deserialize)]
struct LeverJob {
    text: String,
    #[serde(rename = "hostedUrl")]
    hosted_url: String,
    categories: LeverCategories,
}

#[derive(Deserialize)]
struct LeverCategories {
    location: Option<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct SyncResult {
    scanned: usize,
    matched: usize,
    message: &'static str,
}

pub(crate) async fn sync(State(state): State<AppState>) -> Result<Json<SyncResult>, ApiError> {
    let body = reqwest::get(JOBS_URL)
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| {
            eprintln!("Jeeves jobs request failed: {error}");
            ApiError(
                axum::http::StatusCode::BAD_GATEWAY,
                "Could not fetch Jeeves jobs.".into(),
            )
        })?
        .text()
        .await
        .map_err(|error| {
            eprintln!("Jeeves jobs response could not be read: {error}");
            ApiError(
                axum::http::StatusCode::BAD_GATEWAY,
                "Could not read Jeeves jobs.".into(),
            )
        })?;
    let raw: serde_json::Value = serde_json::from_str(&body).map_err(|error| {
        eprintln!("Jeeves jobs response was invalid: {error}");
        ApiError(
            axum::http::StatusCode::BAD_GATEWAY,
            "Could not parse Jeeves jobs.".into(),
        )
    })?;
    let response: Vec<LeverJob> = serde_json::from_value(raw.clone()).map_err(|error| {
        eprintln!("Jeeves jobs response was invalid: {error}");
        ApiError(
            axum::http::StatusCode::BAD_GATEWAY,
            "Could not parse Jeeves jobs.".into(),
        )
    })?;
    let raw_jobs = raw.as_array().ok_or_else(|| {
        ApiError(
            axum::http::StatusCode::BAD_GATEWAY,
            "Could not parse Jeeves jobs.".into(),
        )
    })?;

    let scanned = response.len();
    let normalized: Vec<_> = response
        .iter()
        .zip(raw_jobs)
        .map(|(job, raw_job)| {
            let normalized = crate::job_postings::JobPosting::basic(
                "lever",
                &job.hosted_url,
                job.text.clone(),
                "Jeeves",
                job.categories.location.clone(),
                None,
                job.hosted_url.clone(),
                raw_job,
            );
            normalized
        })
        .collect();
    crate::job_postings::replace_source_snapshot(&state.db, "Jeeves", "lever", &normalized).await?;
    let matched = scanned;

    Ok(Json(SyncResult {
        scanned,
        matched,
        message: "Jeeves snapshot updated from its official Lever postings feed.",
    }))
}
