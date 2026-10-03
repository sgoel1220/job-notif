use super::{
    array_at, bool_at, dedupe, get_json, invalid_feed, percent_encode_component, required_string,
    string_at, workplace_type, AtsCompanySource, AtsFetchError, AtsSourceIdentity,
    NormalizedAtsJob,
};
use serde_json::Value;

pub(super) async fn fetch_bamboohr(
    client: &reqwest::Client,
    company: &AtsCompanySource,
    identity: &AtsSourceIdentity,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let slug = company.slug()?;
    let url = format!(
        "https://{}.bamboohr.com/careers/list",
        percent_encode_component(slug)
    );
    let data = get_json(client, identity, &url).await?;
    map_bamboohr_jobs(identity, &url, &data, slug)
}

pub(super) fn map_bamboohr_jobs(
    identity: &AtsSourceIdentity,
    url: &str,
    data: &Value,
    slug: &str,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let jobs = array_at(data, &["result"])
        .ok_or_else(|| invalid_feed(identity, url, "missing result array in BambooHR feed"))?;
    jobs.iter()
        .map(|job| {
            let mut locations = [
                &["location", "city"][..],
                &["location", "state"][..],
                &["atsLocation", "city"][..],
                &["atsLocation", "province"][..],
                &["atsLocation", "state"][..],
                &["atsLocation", "country"][..],
            ]
            .iter()
            .filter_map(|path| string_at(job, path))
            .collect::<Vec<_>>();
            dedupe(&mut locations);
            let id = required_string(identity, url, job, &["id"], "id")?;
            Ok(NormalizedAtsJob {
                id: id.clone(),
                title: string_at(job, &["jobOpeningName"])
                    .map(|title| title.trim().to_owned())
                    .filter(|title| !title.is_empty())
                    .unwrap_or_else(|| "Untitled role".to_owned()),
                department: string_at(job, &["departmentLabel"]),
                workplace_type: workplace_type(&locations, bool_at(job, &["isRemote"]), false),
                employment_type: string_at(job, &["employmentStatusLabel"]),
                apply_url: format!(
                    "https://{}.bamboohr.com/careers/{}",
                    percent_encode_component(slug),
                    percent_encode_component(&id)
                ),
                posted_at: None,
                locations,
                details: job.clone(),
            })
        })
        .collect()
}
