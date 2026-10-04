use super::{
    array_at, dedupe, ensure_complete_snapshot, get_json, invalid_feed, is_absolute_iso_date,
    next_page_limit, optional_usize, post_json, required_string, string_at, value_to_string,
    workplace_type, AtsCompanySource, AtsFetchError, AtsFetchOptions, AtsSourceIdentity,
    NormalizedAtsJob, WorkdayConfig, WORKDAY_DETAIL_CONCURRENCY, WORKDAY_PAGE_SIZE,
};
use futures_util::{stream, StreamExt};
use serde_json::json;
use serde_json::Value;

pub(super) async fn fetch_workday(
    client: &reqwest::Client,
    company: &AtsCompanySource,
    identity: &AtsSourceIdentity,
    options: &AtsFetchOptions,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let workday = company
        .workday
        .as_ref()
        .ok_or_else(|| AtsFetchError::MissingWorkdayConfig {
            company: company.company_name.clone(),
        })?;
    let base_url = format!(
        "https://{}.{}.myworkdayjobs.com/wday/cxs/{}/{}",
        workday.tenant, workday.shard, workday.tenant, workday.site
    );
    fetch_workday_at(client, identity, options, workday, base_url).await
}

async fn fetch_workday_at(
    client: &reqwest::Client,
    identity: &AtsSourceIdentity,
    options: &AtsFetchOptions,
    workday: &WorkdayConfig,
    base_url: String,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let mut offset = 0usize;
    let mut total: Option<usize> = None;
    let mut postings = Vec::<Value>::new();
    let mut posting_ids = std::collections::HashSet::new();
    loop {
        let Some(limit) =
            next_page_limit(WORKDAY_PAGE_SIZE, postings.len(), options.capped_max_jobs())
        else {
            let total = total.ok_or_else(|| {
                invalid_feed(
                    identity,
                    "workday:pagination",
                    "pagination ended before receiving a total",
                )
            })?;
            ensure_complete_snapshot(identity, "workday:pagination", postings.len(), total)?;
            break;
        };
        let url = format!("{base_url}/jobs");
        let data = post_json(
            client,
            identity,
            &url,
            &json!({ "limit": limit, "offset": offset, "searchText": "" }),
        )
        .await?;
        let reported_total = optional_usize(&data, "total")
            .ok_or_else(|| invalid_feed(identity, &url, "missing or invalid total"))?;
        let page = array_at(&data, &["jobPostings"]).ok_or_else(|| {
            invalid_feed(identity, &url, "missing jobPostings array in Workday page")
        })?;
        total = Some(resolve_workday_page_total(
            identity,
            &url,
            total,
            reported_total,
        )?);
        let total_value = total.unwrap_or_default();
        if page.len() > limit {
            return Err(invalid_feed(
                identity,
                &url,
                format!(
                    "page has {} postings, exceeding requested limit {limit}",
                    page.len()
                ),
            ));
        }
        if postings.len().saturating_add(page.len()) > total_value {
            return Err(invalid_feed(
                identity,
                &url,
                "received more postings than reported total",
            ));
        }
        for posting in page {
            let id = required_string(identity, &url, posting, &["externalPath"], "externalPath")?;
            if !posting_ids.insert(id.clone()) {
                return Err(invalid_feed(
                    identity,
                    &url,
                    format!("duplicate externalPath {id}"),
                ));
            }
            postings.push(posting.clone());
        }
        if page.is_empty() || postings.len() >= total_value {
            ensure_complete_snapshot(identity, &url, postings.len(), total_value)?;
            break;
        }
        offset += page.len();
    }
    ensure_complete_snapshot(
        identity,
        "workday:pagination",
        postings.len(),
        total.ok_or_else(|| invalid_feed(identity, "workday:pagination", "missing total"))?,
    )?;
    let enriched = stream::iter(postings.into_iter().map(|posting| {
        let client = client.clone();
        let identity = identity.clone();
        let base_url = base_url.clone();
        async move {
            let Some(path) = string_at(&posting, &["externalPath"]) else {
                return posting;
            };
            // externalPath already includes `/job/...` in Workday's listing response.
            let detail_url = format!(
                "{base_url}{}",
                if path.starts_with('/') {
                    path.clone()
                } else {
                    format!("/{path}")
                }
            );
            let mut result = Err(String::new());
            for attempt in 0..3 {
                let response = tokio::time::timeout(
                    std::time::Duration::from_secs(15),
                    get_json(&client, &identity, &detail_url),
                )
                .await;
                let retry = match response {
                    Ok(Ok(detail)) => {
                        result = Ok(detail);
                        break;
                    }
                    Ok(Err(error)) => {
                        let retry = match &error {
                            AtsFetchError::Request { .. } => true,
                            AtsFetchError::Http { status, .. } => {
                                status.as_u16() == 429 || status.is_server_error()
                            }
                            _ => false,
                        };
                        result = Err(error.to_string());
                        retry
                    }
                    Err(_) => {
                        result = Err("detail request timed out after 15s".to_owned());
                        true
                    }
                };
                if !retry || attempt == 2 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(250 * (attempt + 1))).await;
            }
            match result {
                Ok(detail) => merge_workday_detail(posting, detail),
                Err(error) => {
                    eprintln!(
                        "Workday detail unavailable; retaining listing for {} {}: {error}",
                        identity.company_name, path
                    );
                    posting
                }
            }
        }
    }))
    .buffered(WORKDAY_DETAIL_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;
    let data = json!({ "jobPostings": enriched });
    map_workday_jobs(identity, "workday:aggregated", &data, workday)
}

fn resolve_workday_page_total(
    identity: &AtsSourceIdentity,
    url: &str,
    expected: Option<usize>,
    reported: usize,
) -> Result<usize, AtsFetchError> {
    match expected {
        None => Ok(reported),
        Some(previous) if reported == previous => Ok(previous),
        // Workday reports the real total on page zero but returns total=0 on later pages.
        // Preserve the first-page snapshot boundary; an empty page before that boundary is
        // still rejected by ensure_complete_snapshot below.
        Some(previous) if reported == 0 && previous > 0 => Ok(previous),
        Some(previous) => Err(invalid_feed(
            identity,
            url,
            format!("total changed from {previous} to {reported}"),
        )),
    }
}

pub(super) fn map_workday_jobs(
    identity: &AtsSourceIdentity,
    url: &str,
    data: &Value,
    workday: &WorkdayConfig,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let postings = array_at(data, &["jobPostings"])
        .ok_or_else(|| invalid_feed(identity, url, "missing jobPostings array in Workday feed"))?;
    postings
        .iter()
        .map(|posting| {
            let external_path =
                required_string(identity, url, posting, &["externalPath"], "externalPath")?;
            let normalized_path = if external_path.starts_with('/') {
                external_path.clone()
            } else {
                format!("/{external_path}")
            };
            let mut locations = string_at(posting, &["locationsText"])
                .into_iter()
                .collect::<Vec<_>>();
            if let Some(info) = posting.get("jobPostingInfo") {
                locations.extend(string_at(info, &["location"]));
                if let Some(additional) = array_at(info, &["additionalLocations"]) {
                    locations.extend(additional.iter().filter_map(value_to_string));
                }
                if let Some(descriptor) = string_at(info, &["jobRequisitionLocation", "descriptor"])
                {
                    locations.push(descriptor);
                }
            }
            dedupe(&mut locations);
            let explicit_remote = match string_at(posting, &["remoteType"]).as_deref() {
                Some("Remote") => Some(true),
                Some("On-Site") | Some("Onsite") => Some(false),
                _ => None,
            };
            Ok(NormalizedAtsJob {
                id: external_path,
                title: string_at(posting, &["title"]).unwrap_or_else(|| "Untitled role".to_owned()),
                department: None,
                workplace_type: workplace_type(&locations, explicit_remote, false),
                employment_type: string_at(posting, &["timeType"]),
                apply_url: format!(
                    "https://{}.{}.myworkdayjobs.com/{}{}",
                    workday.tenant, workday.shard, workday.site, normalized_path
                ),
                posted_at: [
                    ["jobPostingInfo", "startDate"].as_slice(),
                    &["jobPostingInfo", "postedAt"],
                    &["postedDate"],
                    &["postedAt"],
                ]
                .iter()
                .find_map(|path| string_at(posting, path))
                .filter(|date| is_absolute_iso_date(date)),
                locations,
                details: posting.clone(),
            })
        })
        .collect()
}

pub(super) fn merge_workday_detail(mut posting: Value, detail: Value) -> Value {
    // Workday's detail response wraps authoritative fields under jobPostingInfo. Keep the
    // listing payload intact while exposing that object to the normalizer and stored details.
    if let Some(info) = detail.get("jobPostingInfo").cloned() {
        if let Some(object) = posting.as_object_mut() {
            // The generic posting normalizer consumes these canonical fields. Preserve
            // Workday's original response below as well for auditability.
            if let Some(description) = string_at(&info, &["jobDescription"]) {
                object.insert("description".to_owned(), Value::String(description));
            }
            if let Some(description_text) = string_at(&info, &["jobDescriptionPlain"]) {
                object.insert(
                    "descriptionPlain".to_owned(),
                    Value::String(description_text),
                );
            }
            object.insert("jobPostingInfo".to_owned(), info);
            object.insert("detailResponse".to_owned(), detail);
        }
    }
    posting
}

#[cfg(test)]
#[path = "workday_tests.rs"]
mod tests;
