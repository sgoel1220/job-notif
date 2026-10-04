use super::*;
use crate::ats::AtsProvider;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

fn identity() -> AtsSourceIdentity {
    AtsSourceIdentity {
        provider: AtsProvider::SmartRecruiters,
        company_id: None,
        company_name: "Example".into(),
        source_ref: "example".into(),
    }
}

fn serve(
    detail_status: u16,
    detail_body: &'static str,
) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let mut paths = Vec::new();
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let n = socket.read(&mut request).unwrap();
            let line = String::from_utf8_lossy(&request[..n])
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            paths.push(path.clone());
            let (status, body) = if path.contains("/postings?") {
                (
                    200,
                    r#"{"totalFound":1,"content":[{"id":"job-1","name":"Engineer"}]}"#,
                )
            } else {
                (detail_status, detail_body)
            };
            write!(socket, "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        paths
    });
    (format!("http://{address}/v1"), handle)
}

#[tokio::test]
async fn listing_endpoint_and_details_are_best_effort() {
    let (base, server) = serve(404, "down");
    let jobs = fetch_smartrecruiters_at(
        &reqwest::Client::new(),
        &identity(),
        "Example Co",
        &AtsFetchOptions::default(),
        &base,
    )
    .await
    .unwrap();
    let paths = server.join().unwrap();
    assert!(paths[0].starts_with("/v1/companies/Example%20Co/postings?limit=100&offset=0"));
    assert_eq!(paths[1], "/v1/companies/Example%20Co/postings/job-1");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, "job-1");
}

#[test]
fn sections_object_and_array_and_malformed_details_map_safely() {
    let id = identity();
    for sections in [
        json!({"companyDescription":{"text":"C"},"qualifications":{"text":"Q"},"jobDescription":{"text":"<p>Build</p>"}}),
        json!([{"text":"Build"},{"text":"Skills"}]),
        json!("malformed"),
    ] {
        let jobs = map_smartrecruiters_jobs(&id, "test", &json!({"content":[{"id":"j","name":"Engineer","detail":{"jobAd":{"sections":sections}}}]}), "Example").unwrap();
        assert_eq!(jobs.len(), 1);
        if jobs[0].details.get("description").is_some() {
            assert!(jobs[0].details["description"]
                .as_str()
                .unwrap()
                .contains("Build"));
        }
    }
}
