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

fn is_open_and_accepting(job: &RemoteJob) -> bool {
    job.status.eq_ignore_ascii_case("open") && job.accepting_applications
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
        && eligible_region
        && is_open_and_accepting(job)
}

fn parse_and_normalize(
    body: &str,
) -> Result<(usize, usize, Vec<crate::job_postings::JobPosting>), String> {
    let raw: serde_json::Value = serde_json::from_str(body).map_err(|error| error.to_string())?;
    let response: RemoteResponse =
        serde_json::from_value(raw.clone()).map_err(|error| error.to_string())?;
    let raw_jobs = raw
        .get("jobs")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "Remote response is missing its jobs array".to_owned())?;
    if raw_jobs.len() != response.jobs.len() {
        return Err("Remote response jobs array could not be fully decoded".into());
    }

    let scanned = response.jobs.len();
    let matched = response
        .jobs
        .iter()
        .filter(|job| is_target_role(job))
        .count();
    let mut postings = Vec::new();
    for (job, raw_job) in response.jobs.iter().zip(raw_jobs) {
        if !is_open_and_accepting(job) {
            continue;
        }
        let url = format!("https://apply.remote.com/jobs/{}", job.public_id);
        postings.push(crate::job_postings::JobPosting::basic(
            "remote.com",
            &job.public_id,
            job.title.clone(),
            "Remote",
            Some(job.location.clone()),
            Some(job.workplace.clone()),
            url,
            raw_job,
        ));
    }
    Ok((scanned, matched, postings))
}

pub(crate) async fn sync(
    State(state): State<AppState>,
    client: &reqwest::Client,
) -> Result<Json<SyncResult>, ApiError> {
    let body = client
        .get(JOBS_URL)
        .send()
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
    let (scanned, matched, normalized) = parse_and_normalize(&body).map_err(|error| {
        eprintln!("Remote jobs response was invalid: {error}");
        ApiError(
            axum::http::StatusCode::BAD_GATEWAY,
            "Could not parse Remote jobs.".into(),
        )
    })?;
    crate::job_postings::replace_source_snapshot(&state.db, "Remote", "remote.com", &normalized)
        .await?;

    Ok(Json(SyncResult {
        scanned,
        matched,
        message: "Remote snapshot updated with all open roles accepting applications.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_all_open_categories_and_retires_closed_or_nonaccepting_roles() {
        let body = r#"{"jobs":[
            {"title":"Software Engineer","location":"EMEA","workplace":"remote","status":"open","accepting_applications":true,"public_id":"emea","description":"remote"},
            {"title":"Product Designer","location":"United States","workplace":"hybrid","status":"open","accepting_applications":true,"public_id":"hybrid","description":"design"},
            {"title":"Software Engineer","location":"Global","workplace":"remote","status":"closed","accepting_applications":false,"public_id":"closed","description":"closed"},
            {"title":"Software Engineer","location":"India","workplace":"remote","status":"open","accepting_applications":false,"public_id":"not-accepting","description":"not accepting"},
            {"title":"Software Engineer","location":"United States","workplace":"remote","status":"open","accepting_applications":true,"public_id":"us-open","description":"remote"}
        ]}"#;
        let (scanned, matched, jobs) = parse_and_normalize(body).unwrap();
        assert_eq!(scanned, 5);
        assert_eq!(matched, 0);
        assert_eq!(
            jobs.iter()
                .map(|job| job.source_job_id.as_str())
                .collect::<Vec<_>>(),
            vec!["emea", "hybrid", "us-open"]
        );
        assert_eq!(jobs[0].location.as_deref(), Some("EMEA"));
        assert_eq!(jobs[0].workplace_type.as_deref(), Some("remote"));
        assert_eq!(jobs[1].workplace_type.as_deref(), Some("hybrid"));
    }

    #[test]
    fn malformed_or_truncated_payloads_are_rejected() {
        for body in [
            "",
            "<html>maintenance</html>",
            "{}",
            r#"{"jobs":[{"title":"missing fields"}]}"#,
        ] {
            assert!(parse_and_normalize(body).is_err());
        }
    }
}
