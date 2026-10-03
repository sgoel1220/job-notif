use axum::{extract::State, Json};
use serde::Deserialize;

use crate::{errors::ApiError, AppState};

const JOBS_URL: &str = "https://api.ashbyhq.com/posting-api/job-board/linear";

#[derive(Deserialize)]
struct AshbyResponse {
    jobs: Vec<AshbyJob>,
}

#[derive(Deserialize)]
struct AshbyJob {
    title: String,
    location: String,
    #[serde(rename = "isRemote")]
    is_remote: bool,
    #[serde(rename = "jobUrl")]
    job_url: String,
}

#[derive(serde::Serialize)]
pub(crate) struct SyncResult {
    scanned: usize,
    matched: usize,
    message: &'static str,
}

fn is_target_role(job: &AshbyJob) -> bool {
    let title = job.title.to_lowercase();
    let location = job.location.to_lowercase();
    let software_role = [
        "software",
        "engineer",
        "engineering",
        "developer",
        "devops",
        "sre",
        "quality assurance",
        " qa ",
    ]
    .iter()
    .any(|term| title.contains(term));
    let india_or_global = ["india", "global", "worldwide", "anywhere"]
        .iter()
        .any(|term| location.contains(term));

    software_role && job.is_remote && india_or_global
}

pub(crate) async fn sync(State(state): State<AppState>) -> Result<Json<SyncResult>, ApiError> {
    let body = reqwest::get(JOBS_URL)
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| {
            eprintln!("Linear jobs request failed: {error}");
            ApiError(
                axum::http::StatusCode::BAD_GATEWAY,
                "Could not fetch Linear jobs.".into(),
            )
        })?
        .text()
        .await
        .map_err(|error| {
            eprintln!("Linear jobs response could not be read: {error}");
            ApiError(
                axum::http::StatusCode::BAD_GATEWAY,
                "Could not read Linear jobs.".into(),
            )
        })?;
    let raw: serde_json::Value = serde_json::from_str(&body).map_err(|error| {
        eprintln!("Linear jobs response was invalid: {error}");
        ApiError(
            axum::http::StatusCode::BAD_GATEWAY,
            "Could not parse Linear jobs.".into(),
        )
    })?;
    let response: AshbyResponse = serde_json::from_value(raw.clone()).map_err(|error| {
        eprintln!("Linear jobs response was invalid: {error}");
        ApiError(
            axum::http::StatusCode::BAD_GATEWAY,
            "Could not parse Linear jobs.".into(),
        )
    })?;
    let raw_jobs = raw
        .get("jobs")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            ApiError(
                axum::http::StatusCode::BAD_GATEWAY,
                "Could not parse Linear jobs.".into(),
            )
        })?;

    let scanned = response.jobs.len();
    let normalized: Vec<_> = response
        .jobs
        .iter()
        .zip(raw_jobs)
        .map(|(job, raw_job)| {
            let normalized = crate::job_postings::JobPosting::basic(
                "ashby",
                &job.job_url,
                job.title.clone(),
                "Linear",
                Some(job.location.clone()),
                job.is_remote.then(|| "remote".into()),
                job.job_url.clone(),
                raw_job,
            );
            normalized
        })
        .collect();
    crate::job_postings::replace_source_snapshot(&state.db, "Linear", "ashby", &normalized).await?;
    let matching: Vec<_> = response.jobs.into_iter().filter(is_target_role).collect();
    let matched = matching.len();

    // Replace Linear's snapshot only after a successful, complete fetch and parse.

    Ok(Json(SyncResult {
        scanned,
        matched,
        message: "Linear snapshot updated: remote software roles marked India/global only.",
    }))
}
