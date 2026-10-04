use super::*;
use crate::ats::{AtsFetchOptions, AtsProvider, AtsSourceIdentity, WorkdayConfig};

#[test]
fn detail_description_is_exposed_to_common_normalizer() {
    let listing = json!({
        "title": "Engineer",
        "externalPath": "/job/US/Engineer_123",
        "locationsText": "Remote"
    });
    let detail = json!({
        "jobPostingInfo": {
            "title": "Engineer",
            "jobDescription": "<p>Build things</p>",
            "jobDescriptionPlain": "Build things"
        }
    });
    let merged = merge_workday_detail(listing, detail);
    assert_eq!(merged["description"], "<p>Build things</p>");
    assert_eq!(merged["descriptionPlain"], "Build things");
    assert!(merged["detailResponse"]["jobPostingInfo"]["jobDescription"].is_string());
    let config = WorkdayConfig {
        tenant: "tenant".into(),
        shard: "wd1".into(),
        site: "site".into(),
    };
    let mapped = map_workday_jobs(
        &identity(),
        "test",
        &json!({"jobPostings":[merged]}),
        &config,
    )
    .unwrap();
    let fetched = super::super::to_fetched_jobs(identity(), mapped);
    assert_eq!(
        fetched[0].posting.description.as_deref(),
        Some("<p>Build things</p>")
    );
    assert_eq!(
        fetched[0].posting.description_text.as_deref(),
        Some("Build things")
    );
}

#[test]
fn detail_merge_does_not_discard_listing_when_no_detail_description() {
    let listing = json!({"title":"Engineer", "externalPath":"/job/US/Engineer_123"});
    let merged = merge_workday_detail(listing.clone(), json!({"jobPostingInfo":{"id":"abc"}}));
    assert_eq!(merged["title"], listing["title"]);
    assert_eq!(merged["externalPath"], listing["externalPath"]);
}

#[tokio::test]
async fn fetch_accepts_live_zero_total_on_nonempty_later_page() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let mut post_offsets = Vec::new();
        let mut details = 0;
        for _ in 0..23 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let read = stream.read(&mut chunk).unwrap();
                assert!(read > 0, "connection closed before request completed");
                request.extend_from_slice(&chunk[..read]);
                if let Some(header_end) =
                    request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")?
                                .parse::<usize>()
                                .ok()
                        })
                        .unwrap_or(0);
                    if request.len() >= header_end + 4 + content_length {
                        break;
                    }
                }
            }
            let request = String::from_utf8_lossy(&request);
            let path = request.split_whitespace().nth(1).unwrap();
            let body = if path == "/jobs" {
                let payload = request.split("\r\n\r\n").nth(1).unwrap();
                let offset = serde_json::from_str::<Value>(payload).unwrap()["offset"]
                    .as_u64()
                    .unwrap() as usize;
                post_offsets.push(offset);
                let count = if offset == 0 { 20 } else { 1 };
                let postings = (offset..offset + count)
                    .map(|index| json!({"title":format!("Role {index}"),"externalPath":format!("/job/{index}")}))
                    .collect::<Vec<_>>();
                json!({"total":if offset == 0 { 21 } else { 0 },"jobPostings":postings}).to_string()
            } else {
                details += 1;
                "{}".to_owned()
            };
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        }
        (post_offsets, details)
    });
    let jobs = fetch_workday_at(
        &reqwest::Client::new(),
        &identity(),
        &AtsFetchOptions::default(),
        &WorkdayConfig {
            tenant: "tenant".into(),
            shard: "wd1".into(),
            site: "site".into(),
        },
        format!("http://{address}"),
    )
    .await
    .unwrap();
    let (post_offsets, detail_count) = server.join().unwrap();
    assert_eq!(post_offsets, vec![0, 20]);
    assert_eq!(detail_count, 21);
    assert_eq!(jobs.len(), 21);
}

#[test]
fn later_zero_total_preserves_first_page_snapshot_total() {
    let identity = identity();
    // Live Autodesk (total=36) and Adobe (total=526) feeds return total=0 on offset 20.
    assert_eq!(
        resolve_workday_page_total(&identity, "test", Some(36), 0).unwrap(),
        36
    );
    assert_eq!(
        resolve_workday_page_total(&identity, "test", Some(526), 0).unwrap(),
        526
    );
}

#[test]
fn changed_nonzero_total_is_rejected_as_feed_drift() {
    let error = resolve_workday_page_total(&identity(), "test", Some(36), 37).unwrap_err();
    assert!(error.to_string().contains("total changed from 36 to 37"));
}

#[test]
fn empty_premature_page_remains_an_incomplete_snapshot() {
    let identity = identity();
    let total = resolve_workday_page_total(&identity, "test", Some(36), 0).unwrap();
    let error = ensure_complete_snapshot(&identity, "test", 20, total).unwrap_err();
    assert!(error
        .to_string()
        .contains("incomplete snapshot: fetched 20 of 36"));
}

#[test]
fn terminal_page_count_must_match_first_page_total() {
    let identity = identity();
    let total = resolve_workday_page_total(&identity, "test", Some(36), 0).unwrap();
    assert!(ensure_complete_snapshot(&identity, "test", 36, total).is_ok());
    let error = ensure_complete_snapshot(&identity, "test", 35, total).unwrap_err();
    assert!(error
        .to_string()
        .contains("incomplete snapshot: fetched 35 of 36"));
}

fn identity() -> AtsSourceIdentity {
    AtsSourceIdentity {
        provider: AtsProvider::Workday,
        company_id: None,
        company_name: "test".into(),
        source_ref: "tenant/wd1/site".into(),
    }
}

#[tokio::test]
async fn fetch_uses_external_path_and_retains_jobs_on_permanent_detail_failure() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 8192];
        let n = stream.read(&mut request).unwrap();
        let text = String::from_utf8_lossy(&request[..n]);
        assert!(text.starts_with("POST /jobs "));
        let body = r#"{"total":2,"jobPostings":[{"title":"Has detail","externalPath":"/job/US/ok"},{"title":"No detail","externalPath":"/job/US/missing"}]}"#;
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        drop(stream);
        let mut paths = std::collections::HashSet::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let n = stream.read(&mut request).unwrap();
            let text = String::from_utf8_lossy(&request[..n]);
            let path = text.split_whitespace().nth(1).unwrap().to_owned();
            assert!(paths.insert(path.clone()), "duplicate request: {text}");
            let (status, body) = match path.as_str() {
                "/job/US/ok" => (
                    "200 OK",
                    r#"{"jobPostingInfo":{"jobDescription":"<p>hello</p>","jobDescriptionPlain":"hello"}}"#,
                ),
                "/job/US/missing" => ("404 Not Found", "{}"),
                _ => panic!("unexpected request: {text}"),
            };
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        }
    });
    let client = reqwest::Client::new();
    let config = WorkdayConfig {
        tenant: "tenant".into(),
        shard: "wd1".into(),
        site: "site".into(),
    };
    let jobs = fetch_workday_at(
        &client,
        &identity(),
        &AtsFetchOptions::default(),
        &config,
        format!("http://{address}"),
    )
    .await
    .unwrap();
    server.join().unwrap();
    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[0].details["description"], "<p>hello</p>");
    assert_eq!(jobs[1].title, "No detail");
}
