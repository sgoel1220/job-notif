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
        match total {
            Some(previous) if previous != reported_total => {
                return Err(invalid_feed(
                    identity,
                    &url,
                    format!("total changed from {previous} to {reported_total}"),
                ))
            }
            None => total = Some(reported_total),
            _ => {}
        }
        let total_value = total.unwrap_or_default();
        let page = array_at(&data, &["jobPostings"]).ok_or_else(|| {
            invalid_feed(identity, &url, "missing jobPostings array in Workday page")
        })?;
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
            let detail_url = format!(
                "{base_url}/job{}",
                if path.starts_with('/') {
                    path
                } else {
                    format!("/{path}")
                }
            );
            match get_json(&client, &identity, &detail_url).await {
                Ok(detail) => merge_workday_detail(posting, detail),
                Err(_) => posting, // Summary remains usable; unavailable detail metadata stays unknown.
            }
        }
    }))
    .buffer_unordered(WORKDAY_DETAIL_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;
    let data = json!({ "jobPostings": enriched });
    map_workday_jobs(identity, "workday:aggregated", &data, workday)
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
            object.insert("jobPostingInfo".to_owned(), info);
            object.insert("detailResponse".to_owned(), detail);
        }
    }
    posting
}
