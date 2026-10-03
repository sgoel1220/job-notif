use axum::{extract::State, Json};
use serde_json::Value;

use crate::{errors::ApiError, AppState};

const JOBS_URL: &str =
    "https://raw.githubusercontent.com/ConorsCode/open-jobs-data/main/data/jobs.json";
const SOURCE_PREFIX: &str = "open-jobs-data/";

#[derive(serde::Serialize)]
pub(crate) struct SyncResult {
    scanned: usize,
    message: &'static str,
}

fn non_empty_string<'a>(row: &'a Value, key: &str) -> Result<&'a str, String> {
    row.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("missing or invalid {key}"))
}

fn optional_string(row: &Value, key: &str) -> Result<Option<String>, String> {
    match row.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        _ => Err(format!("invalid {key}")),
    }
}

fn locations(row: &Value) -> Result<Option<String>, String> {
    match row.get("locations") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(location)) => Ok(Some(location.clone())),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "invalid locations entry".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|values| (!values.is_empty()).then(|| values.join("; "))),
        _ => Err("invalid locations".to_owned()),
    }
}

fn optional_bool(row: &Value, key: &str) -> Result<Option<bool>, String> {
    match row.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        _ => Err(format!("invalid {key}")),
    }
}

fn parse_snapshot(body: &str) -> Result<Vec<crate::job_postings::JobPosting>, String> {
    let rows: Vec<Value> = serde_json::from_str(body).map_err(|error| error.to_string())?;
    if rows.iter().any(|row| !row.is_object()) {
        return Err("snapshot contains a non-object row".to_owned());
    }
    rows.iter()
        .map(|row| {
            let company = non_empty_string(row, "company")?;
            // Platform is retained as supplied but no longer determines routing.
            let apply_url = non_empty_string(row, "applyUrl")?;
            // The published feed currently has a small number of null IDs/titles. Use the
            // canonical application URL as a stable ID fallback and retain the raw row below.
            let job_id = row
                .get("jobId")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(apply_url);
            let title = row
                .get("title")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("Untitled role");
            let location = locations(row)?;
            let remote = optional_bool(row, "isRemote")?;
            let workplace_type = if remote == Some(true) {
                Some("remote".to_owned())
            } else {
                let location_lower = location.as_deref().unwrap_or_default().to_lowercase();
                if location_lower.contains("hybrid") {
                    Some("hybrid".to_owned())
                } else if ["on-site", "onsite", "in-office"]
                    .iter()
                    .any(|term| location_lower.contains(term))
                {
                    Some("onsite".to_owned())
                } else {
                    None
                }
            };
            let employment_type = optional_string(row, "employmentType")?;
            let department = optional_string(row, "department")?;
            let posted_at = optional_string(row, "postedAt")?;
            let details_json = serde_json::to_string(row).map_err(|error| error.to_string())?;

            let mut job = crate::job_postings::JobPosting::basic(
                SOURCE_PREFIX,
                job_id,
                title.to_owned(),
                company,
                location,
                workplace_type,
                apply_url.to_owned(),
                &details_json,
            );
            job.employment_type = employment_type;
            job.department = department;
            job.posted_at = posted_at;
            job.details_json = details_json;
            Ok(job)
        })
        .collect()
}

pub(crate) async fn sync(State(state): State<AppState>) -> Result<Json<SyncResult>, ApiError> {
    let response = reqwest::Client::new()
        .get(JOBS_URL)
        .header(
            reqwest::header::USER_AGENT,
            "JobNotifier/0.1 (+open-jobs-data snapshot consumer)",
        )
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| {
            eprintln!("open-jobs-data request failed: {error}");
            ApiError(
                axum::http::StatusCode::BAD_GATEWAY,
                "Could not fetch open-jobs-data jobs.".into(),
            )
        })?
        .text()
        .await
        .map_err(|error| {
            eprintln!("open-jobs-data response could not be read: {error}");
            ApiError(
                axum::http::StatusCode::BAD_GATEWAY,
                "Could not read open-jobs-data jobs.".into(),
            )
        })?;

    let jobs = parse_snapshot(&response).map_err(|error| {
        eprintln!("open-jobs-data response was invalid: {error}");
        ApiError(
            axum::http::StatusCode::BAD_GATEWAY,
            "Could not parse open-jobs-data jobs.".into(),
        )
    })?;
    let scanned = jobs.len();

    // Reconcile only our namespaced source rows, and only after the whole snapshot validated.
    crate::job_postings::replace_source_prefix_snapshot(&state.db, SOURCE_PREFIX, &jobs).await?;

    Ok(Json(SyncResult {
        scanned,
        message: "Job snapshot updated from the daily-refreshed open-jobs-data aggregator.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_fields_and_preserves_full_upstream_row() {
        let body = r#"[{"company":"Example","platform":"greenhouse","jobId":"42","title":"Engineer","department":"Product","locations":["Dublin","Remote - EU"],"isRemote":true,"employmentType":"Full-time","applyUrl":"https://example.test/jobs/42","postedAt":"2026-10-01T10:00:00Z","extra":{"keep":true}}]"#;
        let jobs = parse_snapshot(body).unwrap();
        let job = &jobs[0];
        assert_eq!(job.company, "Example");
        assert_eq!(job.source, "open-jobs-data/");
        assert_eq!(job.source_job_id, "42");
        assert_eq!(job.title, "Engineer");
        assert_eq!(job.location.as_deref(), Some("Dublin; Remote - EU"));
        assert_eq!(job.workplace_type.as_deref(), Some("remote"));
        assert_eq!(job.employment_type.as_deref(), Some("Full-time"));
        assert_eq!(job.department.as_deref(), Some("Product"));
        assert_eq!(job.url, "https://example.test/jobs/42");
        assert_eq!(job.posted_at.as_deref(), Some("2026-10-01T10:00:00Z"));
        assert_eq!(
            serde_json::from_str::<Value>(&job.details_json).unwrap()["extra"]["keep"],
            true
        );
    }

    #[tokio::test]
    async fn reconciliation_deactivates_only_its_own_source_prefix() {
        let db = crate::test_database().await;
        sqlx::query("INSERT INTO job_postings (source, source_job_id, title, company, url) VALUES ('open-jobs-data/lever', 'old', 'Old', 'Example', 'https://example.test/old'), ('greenhouse', 'other', 'Other', 'Example', 'https://example.test/other')")
            .execute(&db).await.unwrap();

        let snapshot = parse_snapshot(r#"[{"company":"Example","platform":"lever","jobId":"new","title":"New","applyUrl":"https://example.test/new"}]"#).unwrap();
        assert!(
            crate::job_postings::replace_source_prefix_snapshot(&db, SOURCE_PREFIX, &snapshot)
                .await
                .is_ok()
        );

        let old_active: bool =
            sqlx::query_scalar("SELECT is_active FROM job_postings WHERE source_job_id = 'old'")
                .fetch_one(&db)
                .await
                .unwrap();
        let other_active: bool =
            sqlx::query_scalar("SELECT is_active FROM job_postings WHERE source_job_id = 'other'")
                .fetch_one(&db)
                .await
                .unwrap();
        let new_active: bool =
            sqlx::query_scalar("SELECT is_active FROM job_postings WHERE source_job_id = 'new'")
                .fetch_one(&db)
                .await
                .unwrap();
        assert!(!old_active);
        assert!(other_active);
        assert!(new_active);
    }

    #[test]
    fn falls_back_for_null_upstream_job_id_and_title() {
        let jobs = parse_snapshot(r#"[{"company":"Example","platform":"lever","jobId":null,"title":null,"applyUrl":"https://example.test/jobs/unique"}]"#).unwrap();
        assert_eq!(jobs[0].source_job_id, "https://example.test/jobs/unique");
        assert_eq!(jobs[0].title, "Untitled role");
    }

    #[test]
    fn rejects_incomplete_snapshot_rows() {
        let body = r#"[{"company":"Example","platform":"lever","jobId":"42","title":"Engineer","applyUrl":"https://example.test/jobs/42"},{"company":"","platform":"lever","jobId":"43","title":"Engineer","applyUrl":"https://example.test/jobs/43"}]"#;
        assert!(parse_snapshot(body).is_err());
        assert!(parse_snapshot("not json").is_err());
        assert!(parse_snapshot("[{}]").is_err());
    }

    #[test]
    fn false_remote_flag_is_not_assumed_onsite() {
        let jobs = parse_snapshot(r#"[{"company":"Example","platform":"ashby","jobId":"42","title":"Engineer","isRemote":false,"applyUrl":"https://example.test/jobs/42"}]"#).unwrap();
        assert_eq!(jobs[0].workplace_type, None);
    }
}
