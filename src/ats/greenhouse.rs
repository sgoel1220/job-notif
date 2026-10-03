use super::{
    array_at, dedupe, first_array_object_string, get_json, invalid_feed, percent_encode_component,
    required_string, string_at, workplace_type, AtsCompanySource, AtsFetchError, AtsSourceIdentity,
    NormalizedAtsJob,
};
use serde_json::Value;

pub(super) async fn fetch_greenhouse(
    client: &reqwest::Client,
    company: &AtsCompanySource,
    identity: &AtsSourceIdentity,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let slug = company.slug()?;
    let url = format!(
        "https://boards-api.greenhouse.io/v1/boards/{}/jobs?content=true",
        percent_encode_component(slug)
    );
    let data = get_json(client, identity, &url).await?;
    map_greenhouse_jobs(identity, &url, &data)
}

pub(super) fn map_greenhouse_jobs(
    identity: &AtsSourceIdentity,
    url: &str,
    data: &Value,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let jobs = array_at(data, &["jobs"])
        .ok_or_else(|| invalid_feed(identity, url, "missing jobs array in Greenhouse feed"))?;
    jobs.iter()
        .map(|job| {
            let mut locations = string_at(job, &["location", "name"])
                .into_iter()
                .collect::<Vec<_>>();
            if let Some(offices) = array_at(job, &["offices"]) {
                locations.extend(
                    offices
                        .iter()
                        .filter_map(|office| string_at(office, &["name"])),
                );
            }
            dedupe(&mut locations);
            let apply_url = required_string(identity, url, job, &["absolute_url"], "absolute_url")?;
            Ok(NormalizedAtsJob {
                id: string_at(job, &["id"]).unwrap_or_else(|| apply_url.clone()),
                title: string_at(job, &["title"]).unwrap_or_else(|| "Untitled role".to_owned()),
                department: first_array_object_string(job, "departments", "name"),
                workplace_type: workplace_type(&locations, None, false),
                employment_type: None,
                apply_url,
                posted_at: string_at(job, &["first_published"]),
                locations,
                details: job.clone(),
            })
        })
        .collect()
}
