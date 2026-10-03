use super::{
    array_at, bool_at, ensure_complete_snapshot, get_json, invalid_feed, next_page_limit,
    optional_usize, percent_encode_component, required_string, string_at, workplace_type,
    AtsCompanySource, AtsFetchError, AtsFetchOptions, AtsSourceIdentity, NormalizedAtsJob,
    DEFAULT_PAGE_SIZE,
};
use serde_json::json;
use serde_json::Value;

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
    map_smartrecruiters_jobs(
        identity,
        "smartrecruiters:aggregated",
        &json!({"content":jobs}),
        slug,
    )
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
                details: job.clone(),
            })
        })
        .collect()
}
