use super::{
    array_at, bool_at, dedupe, get_json, invalid_feed, percent_encode_component, required_string,
    string_at, workplace_type, AtsCompanySource, AtsFetchError, AtsSourceIdentity,
    NormalizedAtsJob,
};
use serde_json::Value;

pub(super) async fn fetch_recruitee(
    client: &reqwest::Client,
    company: &AtsCompanySource,
    identity: &AtsSourceIdentity,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let slug = company.slug()?;
    let url = format!(
        "https://{}.recruitee.com/api/offers/",
        percent_encode_component(slug)
    );
    let data = get_json(client, identity, &url).await?;
    map_recruitee_jobs(identity, &url, &data)
}

pub(super) fn map_recruitee_jobs(
    identity: &AtsSourceIdentity,
    url: &str,
    data: &Value,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let offers = array_at(data, &["offers"])
        .ok_or_else(|| invalid_feed(identity, url, "missing offers array in Recruitee feed"))?;
    offers
        .iter()
        .map(|offer| {
            let mut locations = if let Some(items) = array_at(offer, &["locations"]) {
                items
                    .iter()
                    .filter_map(|item| string_at(item, &["name"]))
                    .collect()
            } else {
                Vec::new()
            };
            if locations.is_empty() {
                locations.extend(string_at(offer, &["location"]));
            }
            dedupe(&mut locations);
            let explicit_remote = bool_at(offer, &["remote"]);
            let apply_url = required_string(identity, url, offer, &["careers_url"], "careers_url")?;
            Ok(NormalizedAtsJob {
                id: string_at(offer, &["id"]).unwrap_or_else(|| apply_url.clone()),
                title: string_at(offer, &["title"]).unwrap_or_else(|| "Untitled role".to_owned()),
                department: string_at(offer, &["department"]),
                workplace_type: workplace_type(
                    &locations,
                    explicit_remote,
                    bool_at(offer, &["hybrid"]) == Some(true),
                ),
                employment_type: string_at(offer, &["employment_type_code"]),
                apply_url,
                posted_at: string_at(offer, &["published_at"]),
                locations,
                details: offer.clone(),
            })
        })
        .collect()
}
