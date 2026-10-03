use super::{
    array_at, ashby_workplace_type, dedupe, get_json, invalid_feed, percent_encode_component,
    string_at, AtsCompanySource, AtsFetchError, AtsSourceIdentity, NormalizedAtsJob,
};
use serde_json::Value;

pub(super) async fn fetch_ashby(
    client: &reqwest::Client,
    company: &AtsCompanySource,
    identity: &AtsSourceIdentity,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let slug = company.slug()?;
    let url = format!(
        "https://api.ashbyhq.com/posting-api/job-board/{}?includeCompensation=true",
        percent_encode_component(slug)
    );
    let data = get_json(client, identity, &url).await?;
    map_ashby_jobs(identity, &url, &data)
}

pub(super) fn map_ashby_jobs(
    identity: &AtsSourceIdentity,
    url: &str,
    data: &Value,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let jobs = array_at(data, &["jobs"])
        .ok_or_else(|| invalid_feed(identity, url, "missing jobs array in Ashby feed"))?;
    jobs.iter()
        .map(|job| {
            let mut locations = string_at(job, &["location"])
                .into_iter()
                .collect::<Vec<_>>();
            if let Some(secondaries) = array_at(job, &["secondaryLocations"]) {
                locations.extend(
                    secondaries
                        .iter()
                        .filter_map(|item| string_at(item, &["location"])),
                );
            }
            dedupe(&mut locations);
            let apply_url = string_at(job, &["applyUrl"])
                .or_else(|| string_at(job, &["jobUrl"]))
                .ok_or_else(|| invalid_feed(identity, url, "Ashby job missing applyUrl/jobUrl"))?;
            Ok(NormalizedAtsJob {
                id: string_at(job, &["id"]).unwrap_or_else(|| apply_url.clone()),
                title: string_at(job, &["title"])
                    .map(|title| title.trim().to_owned())
                    .filter(|title| !title.is_empty())
                    .unwrap_or_else(|| "Untitled role".to_owned()),
                department: string_at(job, &["department"]),
                workplace_type: ashby_workplace_type(job, &locations),
                employment_type: string_at(job, &["employmentType"]),
                apply_url,
                posted_at: string_at(job, &["publishedAt"]),
                locations,
                details: job.clone(),
            })
        })
        .collect()
}
