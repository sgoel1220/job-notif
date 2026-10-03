use super::{
    array_at, bool_at, dedupe, get_json, invalid_feed, percent_encode_component, required_string,
    string_at, workplace_type, AtsCompanySource, AtsFetchError, AtsSourceIdentity,
    NormalizedAtsJob,
};
use serde_json::Value;

pub(super) async fn fetch_workable(
    client: &reqwest::Client,
    company: &AtsCompanySource,
    identity: &AtsSourceIdentity,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let slug = company.slug()?;
    let url = format!(
        "https://apply.workable.com/api/v1/widget/accounts/{}?details=false",
        percent_encode_component(slug)
    );
    let data = get_json(client, identity, &url).await?;
    map_workable_jobs(identity, &url, &data)
}

pub(super) fn map_workable_jobs(
    identity: &AtsSourceIdentity,
    url: &str,
    data: &Value,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let jobs = array_at(data, &["jobs"])
        .ok_or_else(|| invalid_feed(identity, url, "missing jobs array in Workable feed"))?;
    jobs.iter()
        .map(|job| {
            let mut locations = ["city", "state", "country"]
                .iter()
                .filter_map(|key| string_at(job, &[*key]))
                .collect::<Vec<_>>();
            dedupe(&mut locations);
            let shortcode = required_string(identity, url, job, &["shortcode"], "shortcode")?;
            Ok(NormalizedAtsJob {
                id: shortcode.clone(),
                title: string_at(job, &["title"]).unwrap_or_else(|| "Untitled role".to_owned()),
                department: string_at(job, &["department"]),
                workplace_type: workplace_type(&locations, bool_at(job, &["telecommuting"]), false),
                employment_type: string_at(job, &["employment_type"]),
                apply_url: string_at(job, &["application_url"])
                    .or_else(|| string_at(job, &["shortlink"]))
                    .or_else(|| string_at(job, &["url"]))
                    .unwrap_or_else(|| format!("https://apply.workable.com/j/{shortcode}")),
                posted_at: string_at(job, &["published_on"]),
                locations,
                details: job.clone(),
            })
        })
        .collect()
}
