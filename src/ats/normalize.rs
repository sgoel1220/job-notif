use super::{join_locations, AtsSourceIdentity, FetchedAtsJob, JobPosting};
use serde_json::Value;

#[derive(Clone, Debug)]
pub(super) struct NormalizedAtsJob {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) department: Option<String>,
    pub(super) locations: Vec<String>,
    pub(super) workplace_type: Option<String>,
    pub(super) employment_type: Option<String>,
    pub(super) apply_url: String,
    pub(super) posted_at: Option<String>,
    pub(super) details: Value,
}

pub(super) fn to_fetched_jobs(
    identity: AtsSourceIdentity,
    jobs: Vec<NormalizedAtsJob>,
) -> Vec<FetchedAtsJob> {
    let source_key = identity.source_key();
    jobs.into_iter()
        .map(|job| {
            let workplace_type = job.workplace_type;
            let mut posting = JobPosting::basic(
                &source_key,
                &job.id,
                job.title,
                &identity.company_name,
                None,
                None,
                job.apply_url,
                &job.details,
            );
            let generic_location = posting.location.take();
            let mut merged_locations = job.locations.clone();
            if let Some(generic_location) = generic_location {
                merged_locations.extend(
                    generic_location
                        .split(';')
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                        .map(str::to_owned),
                );
            }
            posting.location = join_locations(&merged_locations);
            posting.workplace_type = workplace_type.or(posting.workplace_type);
            posting.department = job.department.or(posting.department);
            posting.employment_type = job.employment_type.or(posting.employment_type);
            posting.posted_at = job.posted_at.or(posting.posted_at);
            FetchedAtsJob {
                identity: identity.clone(),
                posting,
            }
        })
        .collect()
}
