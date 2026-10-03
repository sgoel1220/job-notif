use super::{
    array_at, get_json, invalid_feed, percent_encode_component, string_at, unix_millis_to_iso,
    value_to_string, workplace_type, AtsCompanySource, AtsFetchError, AtsSourceIdentity,
    NormalizedAtsJob,
};
use serde_json::Value;

pub(super) async fn fetch_lever(
    client: &reqwest::Client,
    company: &AtsCompanySource,
    identity: &AtsSourceIdentity,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let slug = company.slug()?;
    let url = format!(
        "https://api.lever.co/v0/postings/{}?mode=json",
        percent_encode_component(slug)
    );
    let data = get_json(client, identity, &url).await?;
    map_lever_jobs(identity, &url, &data)
}

pub(super) fn map_lever_jobs(
    identity: &AtsSourceIdentity,
    url: &str,
    data: &Value,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let jobs = data
        .as_array()
        .ok_or_else(|| invalid_feed(identity, url, "Lever feed root is not an array"))?;
    jobs.iter()
        .map(|job| {
            let categories = job.get("categories").unwrap_or(&Value::Null);
            let locations: Vec<String> = if let Some(all) = array_at(categories, &["allLocations"])
            {
                all.iter().filter_map(value_to_string).collect()
            } else {
                string_at(categories, &["location"]).into_iter().collect()
            };
            let normalized_workplace =
                string_at(job, &["workplaceType"]).map(|value| value.trim().to_ascii_lowercase());
            let explicit_hybrid = normalized_workplace.as_deref() == Some("hybrid");
            let explicit_remote = match normalized_workplace.as_deref() {
                Some("remote") => Some(true),
                Some("on-site") | Some("onsite") => Some(false),
                _ => None,
            };
            let apply_url = string_at(job, &["applyUrl"])
                .or_else(|| string_at(job, &["hostedUrl"]))
                .ok_or_else(|| {
                    invalid_feed(identity, url, "Lever job missing applyUrl/hostedUrl")
                })?;
            Ok(NormalizedAtsJob {
                id: string_at(job, &["id"]).unwrap_or_else(|| apply_url.clone()),
                title: string_at(job, &["text"]).unwrap_or_else(|| "Untitled role".to_owned()),
                department: string_at(categories, &["department"]),
                workplace_type: workplace_type(&locations, explicit_remote, explicit_hybrid),
                employment_type: string_at(categories, &["commitment"]),
                apply_url,
                posted_at: string_at(job, &["createdAt"]).and_then(|value| {
                    value
                        .parse::<i64>()
                        .ok()
                        .and_then(unix_millis_to_iso)
                        .or(Some(value))
                }),
                locations,
                details: job.clone(),
            })
        })
        .collect()
}
