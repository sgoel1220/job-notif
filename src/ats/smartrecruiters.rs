use super::{
    array_at, bool_at, ensure_complete_snapshot, get_json, invalid_feed, next_page_limit,
    optional_usize, percent_encode_component, required_string, string_at, workplace_type,
    AtsCompanySource, AtsFetchError, AtsFetchOptions, AtsSourceIdentity, NormalizedAtsJob,
    DEFAULT_PAGE_SIZE,
};
use futures_util::{stream, StreamExt};
use serde_json::json;
use serde_json::Value;

#[cfg(test)]
#[path = "smartrecruiters_tests.rs"]
mod detail_tests;

#[cfg(test)]
#[path = "smartrecruiters_provider_tests.rs"]
mod provider_tests;

pub(super) async fn fetch_smartrecruiters(
    client: &reqwest::Client,
    company: &AtsCompanySource,
    identity: &AtsSourceIdentity,
    options: &AtsFetchOptions,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    fetch_smartrecruiters_at(
        client,
        identity,
        company.slug()?,
        options,
        "https://api.smartrecruiters.com/v1",
    )
    .await
}

pub(super) async fn fetch_smartrecruiters_at(
    client: &reqwest::Client,
    identity: &AtsSourceIdentity,
    slug: &str,
    options: &AtsFetchOptions,
    api_base: &str,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let mut offset = 0usize;
    let mut expected_total = None;
    let mut jobs = Vec::<Value>::new();
    let mut ids = std::collections::HashSet::new();
    loop {
        let Some(limit) = next_page_limit(DEFAULT_PAGE_SIZE, jobs.len(), options.capped_max_jobs())
        else {
            let total = expected_total.ok_or_else(|| {
                invalid_feed(
                    identity,
                    "smartrecruiters:pagination",
                    "pagination ended before receiving totalFound",
                )
            })?;
            ensure_complete_snapshot(identity, "smartrecruiters:pagination", jobs.len(), total)?;
            break;
        };
        let url = format!(
            "{api_base}/companies/{}/postings?limit={limit}&offset={offset}",
            percent_encode_component(slug)
        );
        let data = get_json(client, identity, &url).await?;
        let total = optional_usize(&data, "totalFound")
            .ok_or_else(|| invalid_feed(identity, &url, "missing or invalid totalFound"))?;
        match expected_total {
            Some(previous) if previous != total => {
                return Err(invalid_feed(
                    identity,
                    &url,
                    format!("totalFound changed from {previous} to {total}"),
                ))
            }
            None => expected_total = Some(total),
            _ => {}
        }
        let page = array_at(&data, &["content"]).ok_or_else(|| {
            invalid_feed(
                identity,
                &url,
                "missing content array in SmartRecruiters page",
            )
        })?;
        if page.len() > limit {
            return Err(invalid_feed(
                identity,
                &url,
                format!(
                    "page has {} jobs, exceeding requested limit {limit}",
                    page.len()
                ),
            ));
        }
        if jobs.len().saturating_add(page.len()) > total {
            return Err(invalid_feed(
                identity,
                &url,
                "received more jobs than reported totalFound",
            ));
        }
        for job in page {
            let id = required_string(identity, &url, job, &["id"], "id")?;
            if !ids.insert(id.clone()) {
                return Err(invalid_feed(
                    identity,
                    &url,
                    format!("duplicate posting id {id}"),
                ));
            }
            jobs.push(job.clone());
        }
        if page.is_empty() || jobs.len() >= total {
            ensure_complete_snapshot(identity, &url, jobs.len(), total)?;
            break;
        }
        offset += page.len();
    }
    let jobs = stream::iter(jobs.into_iter().map(|mut job| async move {
        let id = string_at(&job, &["id"]);
        if let Some(id) = id {
            let url = format!(
                "{api_base}/companies/{}/postings/{}",
                percent_encode_component(slug),
                percent_encode_component(&id)
            );
            // The listing is authoritative for discovery; detail lookup is best-effort.
            match fetch_detail(client, identity, &url).await {
                Ok(detail) if !detail.is_null() => job["detail"] = detail,
                Ok(_) => eprintln!("SmartRecruiters returned empty detail for {id} at {url}; retaining listing"),
                Err(error) => eprintln!("SmartRecruiters detail unavailable for {id} at {url}: {error}; retaining listing"),
            }
        }
        job
    }))
    .buffered(4)
    .collect::<Vec<_>>()
    .await;
    map_smartrecruiters_jobs(
        identity,
        "smartrecruiters:aggregated",
        &json!({"content":jobs}),
        slug,
    )
}

async fn fetch_detail(
    client: &reqwest::Client,
    identity: &AtsSourceIdentity,
    url: &str,
) -> Result<Value, AtsFetchError> {
    const MAX_ATTEMPTS: usize = 3;
    for attempt in 0..MAX_ATTEMPTS {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            get_json(client, identity, url),
        )
        .await
        .unwrap_or_else(|_| {
            Err(AtsFetchError::Timeout {
                provider: identity.provider,
                company: identity.company_name.clone(),
                timeout: std::time::Duration::from_secs(15),
            })
        });
        match result {
            Err(error) if attempt + 1 < MAX_ATTEMPTS && retryable_detail_error(&error) => {
                tokio::time::sleep(std::time::Duration::from_millis(100 * (attempt as u64 + 1)))
                    .await;
            }
            other => return other,
        }
    }
    unreachable!()
}

fn retryable_detail_error(error: &AtsFetchError) -> bool {
    match error {
        AtsFetchError::Request { .. } | AtsFetchError::Timeout { .. } => true,
        AtsFetchError::Http { status, .. } => status.as_u16() == 429 || status.is_server_error(),
        _ => false,
    }
}

pub(super) fn map_smartrecruiters_jobs(
    identity: &AtsSourceIdentity,
    url: &str,
    data: &Value,
    fallback_slug: &str,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let jobs = array_at(data, &["content"]).ok_or_else(|| {
        invalid_feed(
            identity,
            url,
            "missing content array in SmartRecruiters feed",
        )
    })?;
    jobs.iter()
        .map(|job| {
            let locations = string_at(job, &["location", "fullLocation"])
                .into_iter()
                .collect::<Vec<_>>();
            let id = required_string(identity, url, job, &["id"], "id")?;
            let company_identifier = string_at(job, &["company", "identifier"])
                .unwrap_or_else(|| fallback_slug.to_owned());
            let detail = job.get("detail").unwrap_or(&Value::Null);
            let sections = detail.get("jobAd").and_then(|v| v.get("sections"));
            let section_order = [
                "jobDescription",
                "qualifications",
                "additionalInformation",
                "companyDescription",
            ];
            let description = sections.and_then(|sections| {
                let chunks = if let Some(object) = sections.as_object() {
                    section_order
                        .iter()
                        .filter_map(|key| object.get(*key))
                        .filter_map(|section| string_at(section, &["text"]))
                        .collect::<Vec<_>>()
                } else if let Some(array) = sections.as_array() {
                    array
                        .iter()
                        .filter_map(|section| string_at(section, &["text"]))
                        .collect()
                } else {
                    Vec::new()
                };
                (!chunks.is_empty()).then(|| chunks.join("\n"))
            });
            let details =
                if let Some(description) = description.filter(|text| !text.trim().is_empty()) {
                    let mut value = job.clone();
                    value["description"] = Value::String(description);
                    value
                } else {
                    job.clone()
                };
            Ok(NormalizedAtsJob {
                id: id.clone(),
                title: string_at(job, &["name"]).unwrap_or_else(|| "Untitled role".to_owned()),
                department: string_at(job, &["department", "label"])
                    .or_else(|| string_at(job, &["function", "label"])),
                workplace_type: workplace_type(
                    &locations,
                    bool_at(job, &["location", "remote"]),
                    bool_at(job, &["location", "hybrid"]) == Some(true),
                ),
                employment_type: string_at(job, &["typeOfEmployment", "label"]),
                apply_url: format!(
                    "https://jobs.smartrecruiters.com/{}/{}",
                    percent_encode_component(&company_identifier),
                    percent_encode_component(&id)
                ),
                posted_at: string_at(job, &["releasedDate"]),
                locations,
                details,
            })
        })
        .collect()
}
