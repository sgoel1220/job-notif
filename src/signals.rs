use axum::{extract::State, Json};
use quick_xml::{events::Event, Reader};
use serde::Serialize;

use crate::{errors::ApiError, AppState};

const JOBS_URL: &str = "https://37signals.com/feed/jobs.xml";

#[derive(Default)]
struct Job {
    title: String,
    url: String,
    text: String,
}

#[derive(Serialize)]
pub(crate) struct SyncResult {
    scanned: usize,
    matched: usize,
    message: &'static str,
}

fn parse_feed(xml: &str) -> Result<Vec<Job>, quick_xml::Error> {
    let mut reader = Reader::from_str(xml);
    let mut jobs = Vec::new();
    let mut current: Option<Job> = None;
    let mut field = String::new();
    loop {
        match reader.read_event()? {
            Event::Start(ref e) if e.local_name().as_ref() == b"entry" => {
                current = Some(Job::default())
            }
            Event::End(ref e) if e.local_name().as_ref() == b"entry" => {
                if let Some(job) = current.take() {
                    jobs.push(job);
                }
                field.clear();
            }
            Event::Start(ref e) if current.is_some() => {
                field = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if field == "link" {
                    if let Some(job) = current.as_mut() {
                        for attr in e.attributes().flatten() {
                            if attr.key.local_name().as_ref() == b"href" {
                                job.url = attr.unescape_value()?.into_owned();
                            }
                        }
                    }
                }
            }
            Event::Text(ref e) if current.is_some() => {
                let text = String::from_utf8_lossy(e.as_ref()).into_owned();
                let job = current.as_mut().unwrap();
                if field == "title" {
                    job.title.push_str(&text);
                }
                job.text.push(' ');
                job.text.push_str(&text);
            }
            Event::CData(ref e) if current.is_some() => {
                current
                    .as_mut()
                    .unwrap()
                    .text
                    .push_str(&String::from_utf8_lossy(e.as_ref()));
            }
            Event::End(_) => field.clear(),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(jobs)
}

fn is_target_role(job: &Job) -> bool {
    let title = job.title.to_lowercase();
    let context = job.text.to_lowercase();
    let software = [
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
    let remote = context.contains("remote");
    let eligible = ["india", "global", "worldwide", "anywhere"]
        .iter()
        .any(|term| context.contains(term));
    software && remote && eligible
}

pub(crate) async fn sync(State(state): State<AppState>) -> Result<Json<SyncResult>, ApiError> {
    let response = reqwest::get(JOBS_URL)
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| {
            eprintln!("37signals jobs request failed: {error}");
            ApiError(
                axum::http::StatusCode::BAD_GATEWAY,
                "Could not fetch 37signals jobs.".into(),
            )
        })?;
    let xml = response.text().await.map_err(|error| {
        eprintln!("37signals jobs response failed: {error}");
        ApiError(
            axum::http::StatusCode::BAD_GATEWAY,
            "Could not read 37signals jobs.".into(),
        )
    })?;
    let jobs = parse_feed(&xml).map_err(|error| {
        eprintln!("37signals Atom feed was invalid: {error}");
        ApiError(
            axum::http::StatusCode::BAD_GATEWAY,
            "Could not parse 37signals jobs.".into(),
        )
    })?;
    let scanned = jobs.len();
    let normalized: Vec<_> = jobs
        .iter()
        .map(|job| {
            let mut normalized = crate::job_postings::JobPosting::basic(
                "37signals-atom",
                &job.url,
                job.title.clone(),
                "37signals",
                Some("Remote — worldwide (eligibility applies)".into()),
                Some("remote".into()),
                job.url.clone(),
                serde_json::json!({"feedText": job.text}),
            );
            normalized.description_text = Some(job.text.clone());
            normalized
        })
        .collect();
    crate::job_postings::replace_source_snapshot(
        &state.db,
        "37signals",
        "37signals-atom",
        &normalized,
    )
    .await?;
    let matching: Vec<_> = jobs.into_iter().filter(is_target_role).collect();
    let matched = matching.len();

    Ok(Json(SyncResult {
        scanned,
        matched,
        message: "37signals Atom feed synced: software roles marked remote and India/global only.",
    }))
}
