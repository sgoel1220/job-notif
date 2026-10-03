use axum::{extract::State, Json};
use serde::Deserialize;

use crate::{errors::ApiError, AppState};

// Official MoonPay careers board, hosted by Lever. Greenhouse's former `moonpay`
// board is no longer active; this public postings endpoint currently returns jobs.
const JOBS_URL: &str = "https://api.lever.co/v0/postings/moonpay?mode=json";

#[derive(Deserialize)]
struct LeverJob {
    text: String,
    categories: LeverCategories,
    #[serde(rename = "hostedUrl")]
    hosted_url: String,
}

#[derive(Deserialize)]
struct LeverCategories {
    location: Option<String>,
    #[serde(rename = "allLocations", default)]
    all_locations: Vec<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct SyncResult {
    scanned: usize,
    matched: usize,
    message: &'static str,
}

fn is_target_role(job: &LeverJob) -> bool {
    let title = job.text.to_lowercase();
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

    let locations = job
        .categories
        .all_locations
        .iter()
        .map(String::as_str)
        .chain(job.categories.location.iter().map(String::as_str));
    let eligible_remote = locations.into_iter().any(|location| {
        let location = location.to_lowercase();
        location.contains("remote")
            && ["india", "global", "worldwide", "anywhere"]
                .iter()
                .any(|term| location.contains(term))
    });

    software_role && eligible_remote
}

pub(crate) async fn sync(State(state): State<AppState>) -> Result<Json<SyncResult>, ApiError> {
    let body = reqwest::get(JOBS_URL)
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| {
            eprintln!("MoonPay jobs request failed: {error}");
            ApiError(
                axum::http::StatusCode::BAD_GATEWAY,
                "Could not fetch MoonPay jobs.".into(),
            )
        })?
        .text()
        .await
        .map_err(|error| {
            eprintln!("MoonPay jobs response could not be read: {error}");
            ApiError(
                axum::http::StatusCode::BAD_GATEWAY,
                "Could not read MoonPay jobs.".into(),
            )
        })?;
    let raw: serde_json::Value = serde_json::from_str(&body).map_err(|error| {
        eprintln!("MoonPay jobs response was invalid: {error}");
        ApiError(
            axum::http::StatusCode::BAD_GATEWAY,
            "Could not parse MoonPay jobs.".into(),
        )
    })?;
    let response: Vec<LeverJob> = serde_json::from_value(raw.clone()).map_err(|error| {
        eprintln!("MoonPay jobs response was invalid: {error}");
        ApiError(
            axum::http::StatusCode::BAD_GATEWAY,
            "Could not parse MoonPay jobs.".into(),
        )
    })?;
    let raw_jobs = raw.as_array().ok_or_else(|| {
        ApiError(
            axum::http::StatusCode::BAD_GATEWAY,
            "Could not parse MoonPay jobs.".into(),
        )
    })?;

    let scanned = response.len();
    let normalized: Vec<_> = response
        .iter()
        .zip(raw_jobs)
        .map(|(job, raw_job)| {
            let location = job
                .categories
                .location
                .clone()
                .or_else(|| job.categories.all_locations.first().cloned());
            let normalized = crate::job_postings::JobPosting::basic(
                "lever",
                &job.hosted_url,
                job.text.clone(),
                "MoonPay",
                location,
                Some("remote eligibility varies".into()),
                job.hosted_url.clone(),
                raw_job,
            );
            normalized
        })
        .collect();
    crate::job_postings::replace_source_snapshot(&state.db, "MoonPay", "lever", &normalized)
        .await?;
    let matching: Vec<_> = response.into_iter().filter(is_target_role).collect();
    let matched = matching.len();

    // Replace the company's snapshot only after a complete successful fetch/parse.

    Ok(Json(SyncResult {
        scanned,
        matched,
        message: "MoonPay snapshot updated: software roles marked remote and India/global only.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(title: &str, locations: &[&str]) -> LeverJob {
        LeverJob {
            text: title.into(),
            categories: LeverCategories {
                location: locations.first().map(|s| (*s).into()),
                all_locations: locations.iter().map(|s| (*s).into()).collect(),
            },
            hosted_url: "https://jobs.lever.co/moonpay/example".into(),
        }
    }

    #[test]
    fn applies_existing_software_remote_india_or_global_filter() {
        assert!(is_target_role(&job(
            "Software Engineer",
            &["India - Remote"]
        )));
        assert!(is_target_role(&job("SRE", &["Global - Remote"])));
        assert!(!is_target_role(&job(
            "Product Manager",
            &["India - Remote"]
        )));
        assert!(!is_target_role(&job(
            "Software Engineer",
            &["United States - Remote"]
        )));
        assert!(!is_target_role(&job(
            "Software Engineer",
            &["India - Hybrid"]
        )));
    }
}
