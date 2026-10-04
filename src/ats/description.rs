use super::{
    get_json, percent_encode_component, to_fetched_jobs, AtsCompanySource, AtsFetchError,
    AtsProvider, AtsSourceIdentity,
};
use serde_json::{json, Value};
use std::time::Duration;

/// Fetch and normalize a single known posting without enumerating the company's job feed.
pub(crate) async fn fetch_job_description(
    client: &reqwest::Client,
    company: &AtsCompanySource,
    job_id: &str,
) -> Result<(Option<String>, Option<String>), AtsFetchError> {
    fetch_job_description_at(client, company, job_id, None).await
}

async fn fetch_job_description_at(
    client: &reqwest::Client,
    company: &AtsCompanySource,
    job_id: &str,
    api_base_override: Option<&str>,
) -> Result<(Option<String>, Option<String>), AtsFetchError> {
    let identity = company.identity()?;
    let detail =
        match company.provider {
            AtsProvider::Workday => {
                let workday = company.workday.as_ref().ok_or_else(|| {
                    AtsFetchError::MissingWorkdayConfig {
                        company: company.company_name.clone(),
                    }
                })?;
                let base = api_base_override.map(str::to_owned).unwrap_or_else(|| {
                    format!(
                        "https://{}.{}.myworkdayjobs.com/wday/cxs/{}/{}",
                        workday.tenant, workday.shard, workday.tenant, workday.site
                    )
                });
                let path = if job_id.starts_with('/') {
                    job_id.to_owned()
                } else {
                    format!("/{job_id}")
                };
                let detail =
                    fetch_detail_with_retry(client, &identity, &format!("{base}{path}")).await?;
                let listing = json!({"externalPath": job_id});
                super::merge_workday_detail(listing, detail)
            }
            AtsProvider::SmartRecruiters => {
                let slug = company
                    .slug
                    .as_deref()
                    .filter(|slug| !slug.trim().is_empty())
                    .ok_or_else(|| AtsFetchError::MissingSlug {
                        provider: company.provider,
                        company: company.company_name.clone(),
                    })?;
                let base = api_base_override.unwrap_or("https://api.smartrecruiters.com/v1");
                let url = format!(
                    "{base}/companies/{}/postings/{}",
                    percent_encode_component(slug),
                    percent_encode_component(job_id)
                );
                let detail = fetch_detail_with_retry(client, &identity, &url).await?;
                json!({"id": job_id, "detail": detail})
            }
            provider => return Err(AtsFetchError::UnsupportedProvider(provider.to_string())),
        };

    let normalized = match company.provider {
        AtsProvider::Workday => super::map_workday_jobs(
            &identity,
            "workday:detail",
            &json!({"jobPostings": [detail]}),
            company.workday.as_ref().expect("validated Workday config"),
        )?,
        AtsProvider::SmartRecruiters => super::map_smartrecruiters_jobs(
            &identity,
            "smartrecruiters:detail",
            &json!({"content": [detail]}),
            company
                .slug
                .as_deref()
                .expect("validated SmartRecruiters slug"),
        )?,
        _ => unreachable!("provider validated above"),
    };
    let Some(fetched) = to_fetched_jobs(identity, normalized).into_iter().next() else {
        return Ok((None, None));
    };
    Ok((
        fetched.posting.description,
        fetched.posting.description_text,
    ))
}

async fn fetch_detail_with_retry(
    client: &reqwest::Client,
    identity: &AtsSourceIdentity,
    url: &str,
) -> Result<Value, AtsFetchError> {
    const MAX_ATTEMPTS: usize = 3;
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
    for attempt in 0..MAX_ATTEMPTS {
        let result = tokio::time::timeout(REQUEST_TIMEOUT, get_json(client, identity, url))
            .await
            .unwrap_or_else(|_| {
                Err(AtsFetchError::Timeout {
                    provider: identity.provider,
                    company: identity.company_name.clone(),
                    timeout: REQUEST_TIMEOUT,
                })
            });
        match result {
            Err(error) if attempt + 1 < MAX_ATTEMPTS && retryable(&error) => {
                tokio::time::sleep(Duration::from_millis(200 * (attempt as u64 + 1))).await;
            }
            other => return other,
        }
    }
    unreachable!("retry loop always returns on final attempt")
}

fn retryable(error: &AtsFetchError) -> bool {
    match error {
        AtsFetchError::Request { .. } | AtsFetchError::Timeout { .. } => true,
        AtsFetchError::Http { status, .. } => status.as_u16() == 429 || status.is_server_error(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn serve(body: &'static str) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let n = socket.read(&mut request).unwrap();
            let line = String::from_utf8_lossy(&request[..n])
                .lines()
                .next()
                .unwrap_or("")
                .to_owned();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            line
        });
        (format!("http://{address}"), handle)
    }

    #[tokio::test]
    async fn smartrecruiters_fetches_one_detail_endpoint_not_the_company_feed() {
        let (base, server) =
            serve(r#"{"jobAd":{"sections":{"jobDescription":{"text":"<p>Role</p>"}}}}"#);
        let company = AtsCompanySource {
            company_id: None,
            company_name: "Example".into(),
            provider: AtsProvider::SmartRecruiters,
            slug: Some("example".into()),
            workday: None,
        };
        let (description, text) =
            fetch_job_description_at(&reqwest::Client::new(), &company, "job-7", Some(&base))
                .await
                .unwrap();
        let request = server.join().unwrap();
        assert!(request.contains("/companies/example/postings/job-7 "));
        assert_eq!(description.as_deref(), Some("<p>Role</p>"));
        assert!(text.as_deref().is_some_and(|text| text.contains("Role")));
    }

    #[tokio::test]
    async fn workday_fetches_stored_external_path_directly() {
        let (base, server) = serve(
            r#"{"jobPostingInfo":{"jobDescription":"<p>Work</p>","jobDescriptionPlain":"Work"}}"#,
        );
        let company = AtsCompanySource {
            company_id: None,
            company_name: "Example".into(),
            provider: AtsProvider::Workday,
            slug: None,
            workday: Some(super::super::WorkdayConfig {
                tenant: "tenant".into(),
                shard: "wd5".into(),
                site: "site".into(),
            }),
        };
        let (description, text) = fetch_job_description_at(
            &reqwest::Client::new(),
            &company,
            "/job/example/123",
            Some(&base),
        )
        .await
        .unwrap();
        let request = server.join().unwrap();
        assert!(request.contains("/job/example/123 "));
        assert_eq!(description.as_deref(), Some("<p>Work</p>"));
        assert_eq!(text.as_deref(), Some("Work"));
    }
}
