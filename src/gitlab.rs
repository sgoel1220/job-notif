use axum::{extract::State, Json};
use serde::Deserialize;

use crate::{errors::ApiError, AppState};

const JOBS_URL: &str = "https://boards-api.greenhouse.io/v1/boards/gitlab/jobs?content=true";

#[derive(Deserialize)]
struct GreenhouseResponse {
    jobs: Vec<GreenhouseJob>,
}

#[derive(Deserialize)]
struct GreenhouseJob {
    title: String,
    location: GreenhouseLocation,
    absolute_url: String,
}

#[derive(Deserialize)]
struct GreenhouseLocation {
    name: String,
}

#[derive(serde::Serialize)]
pub(crate) struct SyncResult {
    scanned: usize,
    matched: usize,
    message: &'static str,
}

fn is_target_role(job: &GreenhouseJob) -> bool {
    let title = job.title.to_lowercase();
    let location = job.location.name.to_lowercase();
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
    let remote = location.contains("remote");
    let india_or_global = ["india", "global", "worldwide", "anywhere"]
        .iter()
        .any(|term| location.contains(term));

    software_role && remote && india_or_global
}

pub(crate) async fn sync(State(state): State<AppState>) -> Result<Json<SyncResult>, ApiError> {
    let body = reqwest::get(JOBS_URL)
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| {
            eprintln!("GitLab jobs request failed: {error}");
            ApiError(
                axum::http::StatusCode::BAD_GATEWAY,
                "Could not fetch GitLab jobs.".into(),
            )
        })?
        .text()
        .await
        .map_err(|error| {
            eprintln!("GitLab jobs response could not be read: {error}");
            ApiError(
                axum::http::StatusCode::BAD_GATEWAY,
                "Could not read GitLab jobs.".into(),
            )
        })?;
    let raw: serde_json::Value = serde_json::from_str(&body).map_err(|error| {
        eprintln!("GitLab jobs response was invalid: {error}");
        ApiError(
            axum::http::StatusCode::BAD_GATEWAY,
            "Could not parse GitLab jobs.".into(),
        )
    })?;
    let response: GreenhouseResponse = serde_json::from_value(raw.clone()).map_err(|error| {
        eprintln!("GitLab jobs response was invalid: {error}");
        ApiError(
            axum::http::StatusCode::BAD_GATEWAY,
            "Could not parse GitLab jobs.".into(),
        )
    })?;
    let raw_jobs = raw
        .get("jobs")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            ApiError(
                axum::http::StatusCode::BAD_GATEWAY,
                "Could not parse GitLab jobs.".into(),
            )
        })?;

    let scanned = response.jobs.len();
    let normalized: Vec<_> = response
        .jobs
        .iter()
        .zip(raw_jobs)
        .map(|(job, raw_job)| {
            let normalized = crate::job_postings::JobPosting::basic(
                "greenhouse",
                &job.absolute_url,
                job.title.clone(),
                "GitLab",
                Some(job.location.name.clone()),
                job.location
                    .name
                    .to_lowercase()
                    .contains("remote")
                    .then(|| "remote".into()),
                job.absolute_url.clone(),
                raw_job,
            );
            normalized
        })
        .collect();
    crate::job_postings::replace_source_snapshot(&state.db, "GitLab", "greenhouse", &normalized)
        .await?;
    let matching: Vec<_> = response.jobs.into_iter().filter(is_target_role).collect();
    let matched = matching.len();

    // Replace GitLab's snapshot only after a successful, complete fetch and parse.

    Ok(Json(SyncResult {
        scanned,
        matched,
        message: "GitLab snapshot updated: software roles marked remote and India/global only.",
    }))
}
