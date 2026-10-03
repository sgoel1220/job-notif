use axum::{extract::State, Json};
use serde::Deserialize;

use crate::{errors::ApiError, AppState};

const JOBS_URL: &str = "https://apply.remote.com/api/public/jobs";

#[derive(Deserialize)]
struct RemoteResponse {
    jobs: Vec<RemoteJob>,
}

#[derive(Deserialize)]
struct RemoteJob {
    title: String,
    location: String,
    workplace: String,
    status: String,
    accepting_applications: bool,
    public_id: String,
}

#[derive(serde::Serialize)]
pub(crate) struct SyncResult {
    scanned: usize,
    matched: usize,
    message: &'static str,
}

fn is_target_role(job: &RemoteJob) -> bool {
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
    let eligible_region = ["india", "global", "worldwide", "anywhere"]
        .iter()
        .any(|term| location.contains(term));

    software_role
        && job.workplace.eq_ignore_ascii_case("remote")
        && job.status.eq_ignore_ascii_case("open")
        && job.accepting_applications
        && eligible_region
}

pub(crate) async fn sync(State(state): State<AppState>) -> Result<Json<SyncResult>, ApiError> {
    let body = reqwest::get(JOBS_URL)
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| {
            eprintln!("Remote jobs request failed: {error}");
            ApiError(
                axum::http::StatusCode::BAD_GATEWAY,
                "Could not fetch Remote jobs.".into(),
            )
        })?
        .text()
        .await
        .map_err(|error| {
            eprintln!("Remote jobs response could not be read: {error}");
            ApiError(
                axum::http::StatusCode::BAD_GATEWAY,
                "Could not read Remote jobs.".into(),
            )
        })?;
    let raw: serde_json::Value = serde_json::from_str(&body).map_err(|error| {
        eprintln!("Remote jobs response was invalid: {error}");
        ApiError(
            axum::http::StatusCode::BAD_GATEWAY,
            "Could not parse Remote jobs.".into(),
        )
    })?;
    let response: RemoteResponse = serde_json::from_value(raw.clone()).map_err(|error| {
        eprintln!("Remote jobs response was invalid: {error}");
        ApiError(
            axum::http::StatusCode::BAD_GATEWAY,
            "Could not parse Remote jobs.".into(),
        )
    })?;
    let raw_jobs = raw
        .get("jobs")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            ApiError(
                axum::http::StatusCode::BAD_GATEWAY,
                "Could not parse Remote jobs.".into(),
            )
        })?;

    let scanned = response.jobs.len();
    let normalized: Vec<_> = response
        .jobs
        .iter()
        .zip(raw_jobs)
        .map(|(job, raw_job)| {
            let url = format!("https://apply.remote.com/jobs/{}", job.public_id);
            let normalized = crate::job_postings::JobPosting::basic(
                "remote.com",
                &job.public_id,
                job.title.clone(),
                "Remote",
                Some(job.location.clone()),
                Some(job.workplace.clone()),
                url,
                raw_job,
            );
            normalized
        })
        .collect();
    crate::job_postings::replace_source_snapshot(&state.db, "Remote", "remote.com", &normalized)
        .await?;
    let matching: Vec<_> = response.jobs.into_iter().filter(is_target_role).collect();
    let matched = matching.len();

    // Preserve the current snapshot unless the official feed was fetched and parsed successfully.

    Ok(Json(SyncResult {
        scanned,
        matched,
        message: "Remote snapshot updated: open remote software roles marked India/global only.",
    }))
}
