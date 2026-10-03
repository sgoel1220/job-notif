use std::{fmt, future::Future, str::FromStr, time::Duration};

use futures_util::stream::{self, StreamExt};
use quick_xml::{events::Event, Reader};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::job_postings::JobPosting;

pub(crate) const ATS_SOURCE_PREFIX: &str = "ats/";
const USER_AGENT: &str = "JobNotifier/0.1 (+https://github.com/ConorsCode/open-jobs-data port)";
const DEFAULT_PAGE_SIZE: usize = 100;
const WORKDAY_PAGE_SIZE: usize = 20;
// Hard safety bound, deliberately above the old 500-job truncation point. Hitting this bound
// while the provider reports more results is an error, never a partial-success snapshot.
const MAX_JOBS_PER_COMPANY: usize = 10_000;
const WORKDAY_DETAIL_CONCURRENCY: usize = 8;
const COMPANY_FETCH_TIMEOUT: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum AtsProvider {
    Greenhouse,
    Lever,
    Ashby,
    SmartRecruiters,
    Workable,
    Recruitee,
    Personio,
    BambooHr,
    Workday,
}

impl AtsProvider {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Greenhouse => "greenhouse",
            Self::Lever => "lever",
            Self::Ashby => "ashby",
            Self::SmartRecruiters => "smartrecruiters",
            Self::Workable => "workable",
            Self::Recruitee => "recruitee",
            Self::Personio => "personio",
            Self::BambooHr => "bamboohr",
            Self::Workday => "workday",
        }
    }
}

impl fmt::Display for AtsProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for AtsProvider {
    type Err = AtsFetchError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "greenhouse" => Ok(Self::Greenhouse),
            "lever" => Ok(Self::Lever),
            "ashby" => Ok(Self::Ashby),
            "smartrecruiters" | "smart-recruiters" => Ok(Self::SmartRecruiters),
            "workable" => Ok(Self::Workable),
            "recruitee" => Ok(Self::Recruitee),
            "personio" => Ok(Self::Personio),
            "bamboohr" | "bamboo-hr" => Ok(Self::BambooHr),
            "workday" => Ok(Self::Workday),
            _ => Err(AtsFetchError::UnsupportedProvider(value.to_owned())),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct WorkdayConfig {
    pub(crate) tenant: String,
    pub(crate) site: String,
    pub(crate) shard: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct AtsCompanySource {
    pub(crate) company_id: Option<String>,
    pub(crate) company_name: String,
    pub(crate) provider: AtsProvider,
    /// Provider board slug. Required for every provider except Workday.
    pub(crate) slug: Option<String>,
    pub(crate) workday: Option<WorkdayConfig>,
}

impl AtsCompanySource {
    pub(crate) fn identity(&self) -> Result<AtsSourceIdentity, AtsFetchError> {
        let source_ref = match self.provider {
            AtsProvider::Workday => {
                let workday =
                    self.workday
                        .as_ref()
                        .ok_or_else(|| AtsFetchError::MissingWorkdayConfig {
                            company: self.company_name.clone(),
                        })?;
                format!("{}/{}/{}", workday.tenant, workday.shard, workday.site)
            }
            _ => self.slug()?.to_owned(),
        };
        Ok(AtsSourceIdentity {
            provider: self.provider,
            company_id: self.company_id.clone(),
            company_name: self.company_name.clone(),
            source_ref,
        })
    }

    fn slug(&self) -> Result<&str, AtsFetchError> {
        self.slug
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| AtsFetchError::MissingSlug {
                provider: self.provider,
                company: self.company_name.clone(),
            })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct AtsSourceIdentity {
    pub(crate) provider: AtsProvider,
    pub(crate) company_id: Option<String>,
    pub(crate) company_name: String,
    /// Slug, or `tenant/shard/site` for Workday.
    pub(crate) source_ref: String,
}

impl AtsSourceIdentity {
    pub(crate) fn source_key(&self) -> String {
        format!(
            "{}{}/{}",
            ATS_SOURCE_PREFIX,
            self.provider.as_str(),
            self.source_ref
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AtsFetchOptions {
    pub(crate) max_jobs_per_company: usize,
}

impl Default for AtsFetchOptions {
    fn default() -> Self {
        Self {
            max_jobs_per_company: MAX_JOBS_PER_COMPANY,
        }
    }
}

impl AtsFetchOptions {
    fn capped_max_jobs(&self) -> usize {
        self.max_jobs_per_company.min(MAX_JOBS_PER_COMPANY)
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct FetchedAtsJob {
    pub(crate) identity: AtsSourceIdentity,
    pub(crate) posting: JobPosting,
}

#[derive(Debug)]
pub(crate) enum AtsFetchError {
    UnsupportedProvider(String),
    ClientBuild {
        message: String,
    },
    MissingSlug {
        provider: AtsProvider,
        company: String,
    },
    MissingWorkdayConfig {
        company: String,
    },
    Request {
        provider: AtsProvider,
        company: String,
        url: String,
        message: String,
    },
    Http {
        provider: AtsProvider,
        company: String,
        url: String,
        status: reqwest::StatusCode,
    },
    InvalidFeed {
        provider: AtsProvider,
        company: String,
        url: String,
        message: String,
    },
    Timeout {
        provider: AtsProvider,
        company: String,
        timeout: Duration,
    },
}

impl fmt::Display for AtsFetchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedProvider(provider) => {
                write!(formatter, "unsupported ATS provider {provider}")
            }
            Self::ClientBuild { message } => {
                write!(formatter, "could not build ATS HTTP client: {message}")
            }
            Self::MissingSlug { provider, company } => {
                write!(formatter, "missing {provider} slug for {company}")
            }
            Self::MissingWorkdayConfig { company } => {
                write!(formatter, "missing Workday config for {company}")
            }
            Self::Request {
                provider,
                company,
                url,
                message,
            } => write!(
                formatter,
                "{provider} request failed for {company} at {url}: {message}"
            ),
            Self::Http {
                provider,
                company,
                url,
                status,
            } => write!(
                formatter,
                "{provider} returned HTTP {status} for {company} at {url}"
            ),
            Self::Timeout {
                provider,
                company,
                timeout,
            } => write!(
                formatter,
                "{provider} fetch timed out for {company} after {timeout:?}"
            ),
            Self::InvalidFeed {
                provider,
                company,
                url,
                message,
            } => write!(
                formatter,
                "invalid {provider} feed for {company} at {url}: {message}"
            ),
        }
    }
}

impl std::error::Error for AtsFetchError {}

pub(crate) fn http_client_with_timeout(
    timeout: Duration,
) -> Result<reqwest::Client, AtsFetchError> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(timeout)
        .build()
        .map_err(|error| AtsFetchError::ClientBuild {
            message: error.to_string(),
        })
}

pub(crate) async fn fetch_company_jobs(
    client: &reqwest::Client,
    company: &AtsCompanySource,
) -> Result<Vec<FetchedAtsJob>, AtsFetchError> {
    fetch_company_jobs_with_options(client, company, &AtsFetchOptions::default()).await
}

pub(crate) async fn fetch_company_jobs_with_options(
    client: &reqwest::Client,
    company: &AtsCompanySource,
    options: &AtsFetchOptions,
) -> Result<Vec<FetchedAtsJob>, AtsFetchError> {
    let identity = company.identity()?;
    run_with_deadline(
        COMPANY_FETCH_TIMEOUT,
        identity.provider,
        identity.company_name.clone(),
        async {
            let jobs = match company.provider {
                AtsProvider::Greenhouse => fetch_greenhouse(client, company, &identity).await?,
                AtsProvider::Lever => fetch_lever(client, company, &identity).await?,
                AtsProvider::Ashby => fetch_ashby(client, company, &identity).await?,
                AtsProvider::SmartRecruiters => {
                    fetch_smartrecruiters(client, company, &identity, options).await?
                }
                AtsProvider::Workable => fetch_workable(client, company, &identity).await?,
                AtsProvider::Recruitee => fetch_recruitee(client, company, &identity).await?,
                AtsProvider::Personio => fetch_personio(client, company, &identity).await?,
                AtsProvider::BambooHr => fetch_bamboohr(client, company, &identity).await?,
                AtsProvider::Workday => fetch_workday(client, company, &identity, options).await?,
            };
            Ok(to_fetched_jobs(identity, jobs))
        },
    )
    .await
}

async fn fetch_greenhouse(
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

async fn fetch_lever(
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

async fn fetch_ashby(
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

async fn fetch_smartrecruiters(
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

async fn fetch_smartrecruiters_at(
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

async fn fetch_workable(
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

async fn fetch_recruitee(
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

async fn fetch_personio(
    client: &reqwest::Client,
    company: &AtsCompanySource,
    identity: &AtsSourceIdentity,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let slug = company.slug()?;
    let url = format!(
        "https://{}.jobs.personio.de/xml",
        percent_encode_component(slug)
    );
    let xml = get_text(client, identity, &url).await?;
    map_personio_jobs(identity, &url, &xml)
}

async fn fetch_bamboohr(
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

async fn fetch_workday(
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

async fn get_text(
    client: &reqwest::Client,
    identity: &AtsSourceIdentity,
    url: &str,
) -> Result<String, AtsFetchError> {
    request_text(client, identity, reqwest::Method::GET, url, None).await
}

async fn get_json(
    client: &reqwest::Client,
    identity: &AtsSourceIdentity,
    url: &str,
) -> Result<Value, AtsFetchError> {
    let text = get_text(client, identity, url).await?;
    serde_json::from_str(&text).map_err(|error| invalid_feed(identity, url, error.to_string()))
}

async fn post_json(
    client: &reqwest::Client,
    identity: &AtsSourceIdentity,
    url: &str,
    body: &Value,
) -> Result<Value, AtsFetchError> {
    let text = request_text(client, identity, reqwest::Method::POST, url, Some(body)).await?;
    serde_json::from_str(&text).map_err(|error| invalid_feed(identity, url, error.to_string()))
}

async fn request_text(
    client: &reqwest::Client,
    identity: &AtsSourceIdentity,
    method: reqwest::Method,
    url: &str,
    body: Option<&Value>,
) -> Result<String, AtsFetchError> {
    let mut request = client
        .request(method, url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT, "application/json,text/xml,*/*");
    if let Some(body) = body {
        request = request
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(body);
    }
    let response = request
        .send()
        .await
        .map_err(|error| AtsFetchError::Request {
            provider: identity.provider,
            company: identity.company_name.clone(),
            url: url.to_owned(),
            message: request_error_message(&error),
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(AtsFetchError::Http {
            provider: identity.provider,
            company: identity.company_name.clone(),
            url: url.to_owned(),
            status,
        });
    }
    response
        .text()
        .await
        .map_err(|error| AtsFetchError::Request {
            provider: identity.provider,
            company: identity.company_name.clone(),
            url: url.to_owned(),
            message: request_error_message(&error),
        })
}

fn request_error_message(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        format!("request timed out: {error}")
    } else {
        error.to_string()
    }
}

#[derive(Clone, Debug)]
struct NormalizedAtsJob {
    id: String,
    title: String,
    department: Option<String>,
    locations: Vec<String>,
    workplace_type: Option<String>,
    employment_type: Option<String>,
    apply_url: String,
    posted_at: Option<String>,
    details: Value,
}

fn map_greenhouse_jobs(
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

fn map_lever_jobs(
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

fn map_ashby_jobs(
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

fn map_smartrecruiters_jobs(
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

fn map_workable_jobs(
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

fn map_recruitee_jobs(
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
            let explicit_remote = match bool_at(offer, &["remote"]) {
                Some(true) => Some(true),
                Some(false) => Some(false),
                None => None,
            };
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

fn map_personio_jobs(
    identity: &AtsSourceIdentity,
    url: &str,
    xml: &str,
) -> Result<Vec<NormalizedAtsJob>, AtsFetchError> {
    let positions = parse_personio_xml(identity, url, xml)?;
    Ok(positions
        .into_iter()
        .map(|position| NormalizedAtsJob {
            id: position.id.clone(),
            title: position.name.clone(),
            department: position.department.clone(),
            workplace_type: workplace_type(&position.locations, None, false),
            employment_type: position.employment_type.clone(),
            apply_url: format!(
                "https://{}.jobs.personio.de/job/{}",
                identity.source_ref,
                percent_encode_component(&position.id)
            ),
            posted_at: position.created_at.clone(),
            locations: position.locations.clone(),
            details: json!({
                "id": position.id,
                "name": position.name,
                "department": position.department,
                "employmentType": position.employment_type,
                "locations": position.locations,
                "createdAt": position.created_at,
            }),
        })
        .collect())
}

fn map_bamboohr_jobs(
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

fn map_workday_jobs(
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

fn to_fetched_jobs(identity: AtsSourceIdentity, jobs: Vec<NormalizedAtsJob>) -> Vec<FetchedAtsJob> {
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

#[derive(Default)]
struct PersonioPosition {
    id: String,
    name: String,
    department: Option<String>,
    employment_type: Option<String>,
    locations: Vec<String>,
    created_at: Option<String>,
}

fn parse_personio_xml(
    identity: &AtsSourceIdentity,
    url: &str,
    xml: &str,
) -> Result<Vec<PersonioPosition>, AtsFetchError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut positions = Vec::new();
    let mut current: Option<PersonioPosition> = None;
    let mut stack: Vec<String> = Vec::new();
    let mut root_closed = false;
    let mut field_text = String::new();
    let mut field_tag: Option<String> = None;
    loop {
        let event = reader
            .read_event()
            .map_err(|error| invalid_feed(identity, url, error.to_string()))?;
        match event {
            Event::Start(element) => {
                let tag = String::from_utf8_lossy(element.name().as_ref()).into_owned();
                if stack.is_empty() {
                    if root_closed || tag != "workzag-jobs" {
                        return Err(invalid_feed(
                            identity,
                            url,
                            "expected one workzag-jobs root element",
                        ));
                    }
                } else if stack == ["workzag-jobs"] {
                    if tag != "position" {
                        return Err(invalid_feed(
                            identity,
                            url,
                            "unexpected element beneath Personio root",
                        ));
                    }
                    current = Some(PersonioPosition::default());
                } else if stack == ["workzag-jobs", "position"] {
                    field_tag = Some(tag.clone());
                    field_text.clear();
                } else {
                    return Err(invalid_feed(
                        identity,
                        url,
                        "nested markup inside a Personio text field is invalid",
                    ));
                }
                stack.push(tag);
            }
            Event::Empty(element) => {
                let tag = String::from_utf8_lossy(element.name().as_ref()).into_owned();
                if stack.is_empty() {
                    if root_closed || tag != "workzag-jobs" {
                        return Err(invalid_feed(
                            identity,
                            url,
                            "expected one workzag-jobs root element",
                        ));
                    }
                    root_closed = true;
                } else if stack == ["workzag-jobs"] {
                    if tag != "position" {
                        return Err(invalid_feed(
                            identity,
                            url,
                            "unexpected element beneath Personio root",
                        ));
                    }
                    return Err(invalid_feed(
                        identity,
                        url,
                        "Personio position missing mandatory id/name",
                    ));
                } else if stack == ["workzag-jobs", "position"] {
                    field_tag = Some(tag);
                    field_text.clear();
                    commit_personio_field(
                        identity,
                        url,
                        current.as_mut().ok_or_else(|| {
                            invalid_feed(identity, url, "position field outside position")
                        })?,
                        field_tag.as_deref().unwrap(),
                        "",
                    )?;
                    field_tag = None;
                } else {
                    return Err(invalid_feed(
                        identity,
                        url,
                        "nested empty markup inside Personio field",
                    ));
                }
            }
            Event::Text(text) => {
                let decoded = text
                    .decode()
                    .map_err(|error| invalid_feed(identity, url, error.to_string()))?;
                let decoded = quick_xml::escape::unescape(&decoded)
                    .map_err(|error| invalid_feed(identity, url, error.to_string()))?;
                if let Some(_) = field_tag {
                    field_text.push_str(&decoded);
                } else if !decoded.trim().is_empty() {
                    return Err(invalid_feed(
                        identity,
                        url,
                        "unexpected text outside a Personio field",
                    ));
                }
            }
            Event::CData(text) => {
                let decoded = text
                    .decode()
                    .map_err(|error| invalid_feed(identity, url, error.to_string()))?;
                if field_tag.is_some() {
                    field_text.push_str(&decoded);
                } else if !decoded.trim().is_empty() {
                    return Err(invalid_feed(
                        identity,
                        url,
                        "unexpected CDATA outside a Personio field",
                    ));
                }
            }
            Event::GeneralRef(reference) => {
                let name = reference
                    .decode()
                    .map_err(|error| invalid_feed(identity, url, error.to_string()))?;
                let resolved = if let Some(character) = reference
                    .resolve_char_ref()
                    .map_err(|error| invalid_feed(identity, url, error.to_string()))?
                {
                    character.to_string()
                } else {
                    quick_xml::escape::resolve_predefined_entity(&name)
                        .ok_or_else(|| {
                            invalid_feed(identity, url, format!("undeclared XML entity &{name};"))
                        })?
                        .to_owned()
                };
                if field_tag.is_some() {
                    field_text.push_str(&resolved);
                } else {
                    return Err(invalid_feed(
                        identity,
                        url,
                        "entity reference outside a Personio field",
                    ));
                }
            }
            Event::End(element) => {
                let tag = String::from_utf8_lossy(element.name().as_ref()).into_owned();
                if stack.last().map(String::as_str) != Some(tag.as_str()) {
                    return Err(invalid_feed(
                        identity,
                        url,
                        "mismatched Personio XML closing tag",
                    ));
                }
                if stack.len() == 3 {
                    let position = current.as_mut().ok_or_else(|| {
                        invalid_feed(identity, url, "field close outside Personio position")
                    })?;
                    commit_personio_field(identity, url, position, &tag, field_text.trim())?;
                    field_tag = None;
                    field_text.clear();
                } else if stack.len() == 2 && tag == "position" {
                    let mut position = current.take().ok_or_else(|| {
                        invalid_feed(identity, url, "unexpected Personio position close")
                    })?;
                    dedupe(&mut position.locations);
                    if position.id.trim().is_empty() || position.name.trim().is_empty() {
                        return Err(invalid_feed(
                            identity,
                            url,
                            "Personio position missing mandatory id/name",
                        ));
                    }
                    positions.push(position);
                } else if stack.len() == 1 && tag == "workzag-jobs" {
                    root_closed = true;
                }
                stack.pop();
            }
            Event::Eof => break,
            Event::Decl(_) | Event::Comment(_) | Event::PI(_) => {}
            Event::DocType(_) => {
                return Err(invalid_feed(
                    identity,
                    url,
                    "DOCTYPE is not allowed in Personio XML",
                ))
            }
        }
    }
    if !root_closed || !stack.is_empty() || current.is_some() {
        return Err(invalid_feed(
            identity,
            url,
            "incomplete Personio XML document",
        ));
    }
    Ok(positions)
}

fn commit_personio_field(
    identity: &AtsSourceIdentity,
    url: &str,
    position: &mut PersonioPosition,
    tag: &str,
    value: &str,
) -> Result<(), AtsFetchError> {
    let value = value.trim();
    match tag {
        "id" => position.id = value.to_owned(),
        "name" => position.name = value.to_owned(),
        "department" if !value.is_empty() => position.department = Some(value.to_owned()),
        "employmentType" if !value.is_empty() => position.employment_type = Some(value.to_owned()),
        "createdAt" if !value.is_empty() => position.created_at = Some(value.to_owned()),
        "office" if !value.is_empty() => position.locations.extend(
            value
                .split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned),
        ),
        _ => {}
    }
    let _ = (identity, url);
    Ok(())
}

fn merge_workday_detail(mut posting: Value, detail: Value) -> Value {
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

fn invalid_feed(
    identity: &AtsSourceIdentity,
    url: impl Into<String>,
    message: impl Into<String>,
) -> AtsFetchError {
    AtsFetchError::InvalidFeed {
        provider: identity.provider,
        company: identity.company_name.clone(),
        url: url.into(),
        message: message.into(),
    }
}

fn required_string(
    identity: &AtsSourceIdentity,
    url: &str,
    value: &Value,
    path: &[&str],
    label: &str,
) -> Result<String, AtsFetchError> {
    string_at(value, path).ok_or_else(|| invalid_feed(identity, url, format!("missing {label}")))
}

fn array_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Vec<Value>> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_array()
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    value_to_string(current)
}

fn bool_at(value: &Value, path: &[&str]) -> Option<bool> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_bool()
}

fn optional_usize(value: &Value, key: &str) -> Option<usize> {
    value.get(key).and_then(|value| match value {
        Value::Number(number) => number
            .as_u64()
            .and_then(|number| usize::try_from(number).ok()),
        Value::String(text) => text.parse().ok(),
        _ => None,
    })
}

fn first_array_object_string(value: &Value, key: &str, object_key: &str) -> Option<String> {
    value
        .get(key)?
        .as_array()?
        .iter()
        .find_map(|item| string_at(item, &[object_key]))
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn ashby_workplace_type(job: &Value, locations: &[String]) -> Option<String> {
    match string_at(job, &["workplaceType"])
        .map(|v| v.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("hybrid") => Some("hybrid".to_owned()),
        Some("remote") => Some("remote".to_owned()),
        Some("onsite") | Some("on-site") | Some("on site") => Some("onsite".to_owned()),
        _ => workplace_type(locations, bool_at(job, &["isRemote"]), false),
    }
}

fn workplace_type(
    locations: &[String],
    explicit_remote: Option<bool>,
    explicit_hybrid: bool,
) -> Option<String> {
    let location_text = locations.join(" ").to_ascii_lowercase();
    if explicit_hybrid || (explicit_remote.is_none() && location_text.contains("hybrid")) {
        Some("hybrid".to_owned())
    } else if explicit_remote == Some(true)
        || (explicit_remote.is_none() && location_text.contains("remote"))
    {
        Some("remote".to_owned())
    } else if explicit_remote == Some(false) {
        Some("onsite".to_owned())
    } else {
        None
    }
}

fn join_locations(locations: &[String]) -> Option<String> {
    let mut values = locations
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    dedupe(&mut values);
    (!values.is_empty()).then(|| values.join("; "))
}

fn dedupe(values: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::<String>::new();
    values.retain(|value| seen.insert(value.to_ascii_lowercase()));
}

fn is_absolute_iso_date(value: &str) -> bool {
    let date = value.get(..10).unwrap_or("");
    let bytes = date.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
        || (value.len() > 10 && !matches!(value.as_bytes()[10], b'T' | b't' | b' '))
    {
        return false;
    }
    if value.len() > 10 {
        let tail = &value[11..];
        if tail.len() < 8
            || !tail.as_bytes()[0..2].iter().all(u8::is_ascii_digit)
            || tail.as_bytes()[2] != b':'
            || !tail.as_bytes()[3..5].iter().all(u8::is_ascii_digit)
            || tail.as_bytes()[5] != b':'
            || !tail.as_bytes()[6..8].iter().all(u8::is_ascii_digit)
        {
            return false;
        }
        let hour: u8 = tail[0..2].parse().unwrap_or(255);
        let minute: u8 = tail[3..5].parse().unwrap_or(255);
        let second: u8 = tail[6..8].parse().unwrap_or(255);
        if hour > 23 || minute > 59 || second > 60 {
            return false;
        }
        let suffix = &tail[8..];
        let suffix = if let Some(fraction) = suffix.strip_prefix('.') {
            let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 {
                return false;
            }
            &fraction[digits..]
        } else {
            suffix
        };
        if suffix != "Z" && suffix != "z" {
            if suffix.len() != 6
                || !matches!(suffix.as_bytes()[0], b'+' | b'-')
                || suffix.as_bytes()[3] != b':'
                || !suffix.as_bytes()[1..3].iter().all(u8::is_ascii_digit)
                || !suffix.as_bytes()[4..6].iter().all(u8::is_ascii_digit)
            {
                return false;
            }
            let tz_hour: u8 = suffix[1..3].parse().unwrap_or(255);
            let tz_minute: u8 = suffix[4..6].parse().unwrap_or(255);
            if tz_hour > 23 || tz_minute > 59 {
                return false;
            }
        }
    }
    let year: u32 = date[..4].parse().unwrap_or(0);
    let month: u32 = date[5..7].parse().unwrap_or(0);
    let day: u32 = date[8..10].parse().unwrap_or(0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    };
    day >= 1 && day <= days
}

fn ensure_complete_snapshot(
    identity: &AtsSourceIdentity,
    url: &str,
    fetched: usize,
    total: usize,
) -> Result<(), AtsFetchError> {
    if fetched < total {
        Err(invalid_feed(
            identity,
            url,
            format!("incomplete snapshot: fetched {fetched} of {total}"),
        ))
    } else {
        Ok(())
    }
}

async fn run_with_deadline<T, F>(
    timeout: Duration,
    provider: AtsProvider,
    company: String,
    future: F,
) -> Result<T, AtsFetchError>
where
    F: Future<Output = Result<T, AtsFetchError>>,
{
    tokio::time::timeout(timeout, future)
        .await
        .map_err(|_| AtsFetchError::Timeout {
            provider,
            company,
            timeout,
        })?
}

fn next_page_limit(page_size: usize, current_jobs: usize, max_jobs: usize) -> Option<usize> {
    if current_jobs >= max_jobs || page_size == 0 || max_jobs == 0 {
        return None;
    }
    Some(page_size.min(max_jobs - current_jobs))
}

// XML text and CDATA decoding is handled event-by-event in parse_personio_xml.

fn percent_encode_component(input: &str) -> String {
    let mut encoded = String::new();
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(char::from(byte));
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn unix_millis_to_iso(milliseconds: i64) -> Option<String> {
    let seconds = milliseconds.div_euclid(1_000);
    let millis = milliseconds.rem_euclid(1_000);
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let z = days.checked_add(719_468)?;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let hour = day_seconds / 3_600;
    let minute = day_seconds % 3_600 / 60;
    let second = day_seconds % 60;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(provider: AtsProvider) -> AtsSourceIdentity {
        AtsSourceIdentity {
            provider,
            company_id: Some("company-1".to_owned()),
            company_name: "Example".to_owned(),
            source_ref: "example".to_owned(),
        }
    }

    fn mock_smart_server<F>(
        requests: usize,
        responder: F,
    ) -> (String, std::thread::JoinHandle<Vec<(usize, usize)>>)
    where
        F: Fn(usize, usize) -> String + Send + 'static,
    {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..requests {
                let (mut socket, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut buf = [0u8; 1024];
                loop {
                    let count = socket.read(&mut buf).unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buf[..count]);
                    if request.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let first = String::from_utf8_lossy(&request)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_owned();
                let url = first.split_whitespace().nth(1).unwrap_or("/");
                let query = url.split_once('?').map(|(_, q)| q).unwrap_or("");
                let get = |key: &str| {
                    query
                        .split('&')
                        .find_map(|p| p.strip_prefix(&format!("{key}=")))
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0)
                };
                let (limit, offset) = (get("limit"), get("offset"));
                seen.push((offset, limit));
                let body = responder(offset, limit);
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            }
            seen
        });
        (format!("http://{address}/v1"), handle)
    }

    #[test]
    fn maps_greenhouse_fixture() {
        let data = json!({
            "jobs": [{
                "id": 42,
                "title": "Platform Engineer",
                "absolute_url": "https://boards.greenhouse.io/example/jobs/42",
                "first_published": "2026-10-02T11:31:50-04:00",
                "location": {"name": "Remote - EU"},
                "departments": [{"name": "Engineering"}]
            }]
        });
        let identity = identity(AtsProvider::Greenhouse);
        let fetched = to_fetched_jobs(
            identity.clone(),
            map_greenhouse_jobs(&identity, "fixture", &data).unwrap(),
        );
        let job = &fetched[0].posting;
        assert_eq!(job.source, "ats/greenhouse/example");
        assert_eq!(job.company, "Example");
        assert_eq!(job.source_job_id, "42");
        assert_eq!(job.title, "Platform Engineer");
        assert_eq!(job.department.as_deref(), Some("Engineering"));
        assert_eq!(job.location.as_deref(), Some("Remote - EU"));
        assert_eq!(job.workplace_type.as_deref(), Some("remote"));
        assert_eq!(job.url, "https://boards.greenhouse.io/example/jobs/42");
        assert_eq!(job.posted_at.as_deref(), Some("2026-10-02T11:31:50-04:00"));
    }

    #[test]
    fn maps_lever_fixture_and_millis_date() {
        let data = json!([{
            "id": "abc",
            "text": "Backend Engineer",
            "applyUrl": "https://jobs.lever.co/example/abc",
            "createdAt": 1751559111614i64,
            "workplaceType": "hybrid",
            "categories": {
                "department": "Product Engineering",
                "commitment": "Full-time",
                "allLocations": ["London", "Remote - UK"]
            }
        }]);
        let identity = identity(AtsProvider::Lever);
        let fetched = to_fetched_jobs(
            identity.clone(),
            map_lever_jobs(&identity, "fixture", &data).unwrap(),
        );
        let job = &fetched[0].posting;
        assert_eq!(job.source_job_id, "abc");
        assert_eq!(job.title, "Backend Engineer");
        assert_eq!(job.location.as_deref(), Some("London; Remote - UK"));
        assert_eq!(job.employment_type.as_deref(), Some("Full-time"));
        assert_eq!(job.department.as_deref(), Some("Product Engineering"));
        assert_eq!(job.posted_at.as_deref(), Some("2025-07-03T16:11:51.614Z"));
    }

    #[test]
    fn maps_personio_xml_fixture() {
        let xml = r#"
            <workzag-jobs>
              <position>
                <id>123</id>
                <name>R&amp;D Engineer</name>
                <department>Engineering</department>
                <employmentType>permanent</employmentType>
                <office>Berlin, Remote - EU</office>
                <createdAt>2026-10-01T00:00:00+00:00</createdAt>
              </position>
            </workzag-jobs>
        "#;
        let identity = identity(AtsProvider::Personio);
        let fetched = to_fetched_jobs(
            identity.clone(),
            map_personio_jobs(&identity, "fixture", xml).unwrap(),
        );
        let job = &fetched[0].posting;
        assert_eq!(job.source, "ats/personio/example");
        assert_eq!(job.source_job_id, "123");
        assert_eq!(job.title, "R&D Engineer");
        assert_eq!(job.location.as_deref(), Some("Berlin; Remote - EU"));
        assert_eq!(job.workplace_type.as_deref(), Some("remote"));
        assert_eq!(job.employment_type.as_deref(), Some("permanent"));
        assert_eq!(job.url, "https://example.jobs.personio.de/job/123");
    }

    #[test]
    fn personio_validates_document_and_decodes_cdata() {
        let identity = identity(AtsProvider::Personio);
        let xml = r#"<workzag-jobs><position><id><![CDATA[a&b/42]]></id><name><![CDATA[R&D Engineer]]></name><office><![CDATA[Berlin, India]]></office></position></workzag-jobs>"#;
        let mapped = map_personio_jobs(&identity, "fixture", xml).unwrap();
        assert_eq!(mapped.len(), 1);
        assert_eq!(mapped[0].id, "a&b/42");
        assert_eq!(mapped[0].title, "R&D Engineer");
        assert_eq!(mapped[0].locations, ["Berlin", "India"]);
        let fetched = to_fetched_jobs(identity.clone(), mapped);
        assert_eq!(
            fetched[0].posting.url,
            "https://example.jobs.personio.de/job/a%26b%2F42"
        );
        assert!(map_personio_jobs(
            &identity,
            "fixture",
            "<workzag-jobs><position><id>42</id><name>Engineer</name>"
        )
        .is_err());
        assert!(map_personio_jobs(&identity, "fixture", "<wrong-root/>").is_err());
        assert!(map_personio_jobs(
            &identity,
            "fixture",
            "<workzag-jobs><position><id>42</id></position></workzag-jobs>"
        )
        .is_err());
        assert!(map_personio_jobs(&identity, "fixture", "<workzag-jobs/>")
            .unwrap()
            .is_empty());
        assert_eq!(
            map_personio_jobs(
                &identity,
                "fixture",
                "<workzag-jobs><position><id>7</id><name>A&amp;B</name></position></workzag-jobs>"
            )
            .unwrap()[0]
                .title,
            "A&B"
        );
        assert_eq!(
            map_personio_jobs(&identity, "fixture", "<workzag-jobs><position><id>7</id><name><![CDATA[A&amp;B]]></name></position></workzag-jobs>").unwrap()[0].title,
            "A&amp;B"
        );
        for malformed in [
            "<workzag-jobs><position><id>7</id><name>A &bogus; B</name></position></workzag-jobs>",
            "<workzag-jobs><position><id>7</id><name><b>Engineer</b></name></position></workzag-jobs>",
            "<workzag-jobs><position><id>7</id><name>Engineer</name></position></workzag-jobs>trailing",
            "<workzag-jobs/><workzag-jobs/>",
        ] { assert!(map_personio_jobs(&identity, "fixture", malformed).is_err(), "accepted {malformed}"); }
    }

    #[test]
    fn explicit_hybrid_overrides_remote_flags() {
        let ashby_id = identity(AtsProvider::Ashby);
        for remote in [true, false] {
            let data = json!({"jobs": [{"id":"a", "title":"Role", "applyUrl":"https://apply", "workplaceType":"Hybrid", "isRemote":remote, "location":"Canada - Remote"}]});
            let posting = &to_fetched_jobs(
                ashby_id.clone(),
                map_ashby_jobs(&ashby_id, "fixture", &data).unwrap(),
            )[0]
            .posting;
            assert_eq!(posting.workplace_type.as_deref(), Some("hybrid"));
        }
        for (workplace, expected) in [
            ("Remote", "remote"),
            ("OnSite", "onsite"),
            ("On-site", "onsite"),
        ] {
            let data = json!({"jobs": [{"id":"a", "title":"Role", "applyUrl":"https://apply", "workplaceType":workplace, "isRemote":false, "location":"In-office"}]});
            let posting = &to_fetched_jobs(
                ashby_id.clone(),
                map_ashby_jobs(&ashby_id, "fixture", &data).unwrap(),
            )[0]
            .posting;
            assert_eq!(posting.workplace_type.as_deref(), Some(expected));
        }
        let lever_id = identity(AtsProvider::Lever);
        let data = json!([{"id":"l", "text":"Role", "applyUrl":"https://apply", "workplaceType":"hybrid", "categories":{"allLocations":["Canada - Remote"]}}]);
        let posting = &to_fetched_jobs(
            lever_id.clone(),
            map_lever_jobs(&lever_id, "fixture", &data).unwrap(),
        )[0]
        .posting;
        assert_eq!(posting.workplace_type.as_deref(), Some("hybrid"));
        let sr_id = identity(AtsProvider::SmartRecruiters);
        let data = json!({"content":[{"id":"s", "name":"Role", "location":{"remote":false,"hybrid":true,"fullLocation":"Toronto"}}]});
        let posting = &to_fetched_jobs(
            sr_id.clone(),
            map_smartrecruiters_jobs(&sr_id, "fixture", &data, "fallback").unwrap(),
        )[0]
        .posting;
        assert_eq!(posting.workplace_type.as_deref(), Some("hybrid"));
    }

    #[test]
    fn preserves_greenhouse_secondary_offices_and_workday_details() {
        let gh = identity(AtsProvider::Greenhouse);
        let data = json!({"jobs":[{"id":1,"title":"Role","absolute_url":"https://apply","location":{"name":"New York"},"offices":[{"name":"India"}]}]});
        let posting = &to_fetched_jobs(
            gh.clone(),
            map_greenhouse_jobs(&gh, "fixture", &data).unwrap(),
        )[0]
        .posting;
        assert_eq!(posting.location.as_deref(), Some("New York; India"));
        let workday = identity(AtsProvider::Workday);
        let summary = json!({"externalPath":"/job/foo","locationsText":"United States","postedOn":"Posted 3 Days Ago"});
        let enriched = merge_workday_detail(
            summary,
            json!({"jobPostingInfo":{"location":"Austin, TX","additionalLocations":["India"],"startDate":"2026-10-01"}}),
        );
        let result = map_workday_jobs(
            &workday,
            "fixture",
            &json!({"jobPostings":[enriched]}),
            &WorkdayConfig {
                tenant: "example".into(),
                site: "External".into(),
                shard: "wd1".into(),
            },
        )
        .unwrap();
        assert_eq!(
            result[0].locations,
            ["United States", "Austin, TX", "India"]
        );
        assert_eq!(result[0].posted_at.as_deref(), Some("2026-10-01"));
        let relative = map_workday_jobs(
            &workday,
            "fixture",
            &json!({"jobPostings":[{"externalPath":"/job/rel","postedOn":"Posted 3 Days Ago"}]}),
            &WorkdayConfig {
                tenant: "example".into(),
                site: "External".into(),
                shard: "wd1".into(),
            },
        )
        .unwrap();
        assert_eq!(relative[0].posted_at, None);
        assert!(is_absolute_iso_date("2026-10-03"));
        assert!(!is_absolute_iso_date("Posted 3 Days Ago"));
    }

    #[tokio::test]
    async fn smartrecruiters_http_pagination_fetches_all_jobs_past_500() {
        let (base, server) = mock_smart_server(6, |offset, limit| {
            let jobs = (offset..(offset + limit).min(501))
                .map(|i| json!({"id":format!("j{i}"),"name":format!("Role {i}")}))
                .collect::<Vec<_>>();
            json!({"totalFound":501,"content":jobs}).to_string()
        });
        let id = identity(AtsProvider::SmartRecruiters);
        let result = fetch_smartrecruiters_at(
            &reqwest::Client::new(),
            &id,
            "test",
            &AtsFetchOptions::default(),
            &base,
        )
        .await
        .unwrap();
        let seen = server.join().unwrap();
        assert_eq!(
            seen.iter().map(|(offset, _)| *offset).collect::<Vec<_>>(),
            [0, 100, 200, 300, 400, 500]
        );
        assert_eq!(result.len(), 501);
        assert_eq!(result[500].id, "j500");
    }

    #[tokio::test]
    async fn smartrecruiters_rejects_missing_total_oversized_pages_and_caps() {
        let id = identity(AtsProvider::SmartRecruiters);
        let (base, server) = mock_smart_server(1, |_, _| json!({"content":[]}).to_string());
        assert!(fetch_smartrecruiters_at(
            &reqwest::Client::new(),
            &id,
            "test",
            &AtsFetchOptions::default(),
            &base
        )
        .await
        .is_err());
        server.join().unwrap();

        let (base, server) = mock_smart_server(1, |_, _| {
            json!({"totalFound":1,"content":[{"name":"No ID"}]}).to_string()
        });
        assert!(fetch_smartrecruiters_at(
            &reqwest::Client::new(),
            &id,
            "test",
            &AtsFetchOptions::default(),
            &base
        )
        .await
        .is_err());
        server.join().unwrap();

        let (base, server) = mock_smart_server(1, |_, _| {
            let content = (0..101)
                .map(|i| json!({"id":format!("{i}"),"name":"Role"}))
                .collect::<Vec<_>>();
            json!({"totalFound":101,"content":content}).to_string()
        });
        assert!(fetch_smartrecruiters_at(
            &reqwest::Client::new(),
            &id,
            "test",
            &AtsFetchOptions::default(),
            &base
        )
        .await
        .is_err());
        server.join().unwrap();

        let (base, server) = mock_smart_server(1, |_, limit| {
            let content = (0..limit)
                .map(|i| json!({"id":format!("{i}"),"name":"Role"}))
                .collect::<Vec<_>>();
            json!({"totalFound":101,"content":content}).to_string()
        });
        let options = AtsFetchOptions {
            max_jobs_per_company: 100,
        };
        assert!(
            fetch_smartrecruiters_at(&reqwest::Client::new(), &id, "test", &options, &base)
                .await
                .is_err()
        );
        server.join().unwrap();
    }

    #[tokio::test]
    async fn smartrecruiters_rejects_duplicate_ids_and_changing_totals() {
        let id = identity(AtsProvider::SmartRecruiters);
        for change_total in [false, true] {
            let (base, server) = mock_smart_server(2, move |offset, limit| {
                let total = if change_total && offset > 0 { 102 } else { 101 };
                let content = if offset == 0 {
                    (0..limit)
                        .map(|i| json!({"id":format!("j{i}"),"name":"Role"}))
                        .collect::<Vec<_>>()
                } else {
                    vec![json!({"id": if change_total { "j100" } else { "j0" },"name":"Role"})]
                };
                json!({"totalFound":total,"content":content}).to_string()
            });
            assert!(fetch_smartrecruiters_at(
                &reqwest::Client::new(),
                &id,
                "test",
                &AtsFetchOptions::default(),
                &base
            )
            .await
            .is_err());
            server.join().unwrap();
        }
    }

    #[tokio::test]
    async fn company_deadline_returns_an_error_without_partial_success() {
        let result: Result<(), AtsFetchError> = run_with_deadline(
            Duration::from_millis(5),
            AtsProvider::Workday,
            "Example".into(),
            std::future::pending(),
        )
        .await;
        assert!(matches!(result, Err(AtsFetchError::Timeout { .. })));
    }

    #[test]
    fn maps_matching_smartrecruiters_job_after_index_500() {
        let id = identity(AtsProvider::SmartRecruiters);
        let jobs = (0..501).map(|index| json!({"id":format!("job-{index}"),"name":format!("Role {index}"),"location":{"fullLocation":"India"}})).collect::<Vec<_>>();
        let mapped =
            map_smartrecruiters_jobs(&id, "fixture", &json!({"content":jobs}), "example").unwrap();
        assert_eq!(mapped.len(), 501);
        assert_eq!(mapped[500].id, "job-500");
    }

    #[test]
    fn pagination_can_continue_past_500_jobs_and_is_bounded() {
        let mut total = 0;
        let mut offsets = Vec::new();
        while let Some(limit) = next_page_limit(100, total, 10_000) {
            offsets.push(total);
            total += if total == 600 { 50 } else { limit };
            if total >= 650 {
                break;
            }
        }
        assert_eq!(offsets, [0, 100, 200, 300, 400, 500, 600]);
        assert_eq!(total, 650);
        assert_eq!(next_page_limit(100, 10_000, 10_000), None);
        let id = identity(AtsProvider::Workday);
        assert!(ensure_complete_snapshot(&id, "fixture", 650, 650).is_ok());
        assert!(ensure_complete_snapshot(&id, "fixture", 500, 650).is_err());
    }

    #[test]
    fn maps_workday_fixture() {
        let data = json!({
            "jobPostings": [{
                "externalPath": "/job/US-Remote/Staff-Engineer_R1",
                "title": "Staff Engineer",
                "locationsText": "United States - Remote",
                "remoteType": "Remote",
                "timeType": "Full time"
            }]
        });
        let workday = WorkdayConfig {
            tenant: "example".to_owned(),
            site: "External".to_owned(),
            shard: "wd1".to_owned(),
        };
        let identity = AtsSourceIdentity {
            provider: AtsProvider::Workday,
            company_id: None,
            company_name: "Example".to_owned(),
            source_ref: "example/wd1/External".to_owned(),
        };
        let fetched = to_fetched_jobs(
            identity.clone(),
            map_workday_jobs(&identity, "fixture", &data, &workday).unwrap(),
        );
        let job = &fetched[0].posting;
        assert_eq!(job.source, "ats/workday/example/wd1/External");
        assert_eq!(job.source_job_id, "/job/US-Remote/Staff-Engineer_R1");
        assert_eq!(job.workplace_type.as_deref(), Some("remote"));
        assert_eq!(job.employment_type.as_deref(), Some("Full time"));
        assert_eq!(
            job.url,
            "https://example.wd1.myworkdayjobs.com/External/job/US-Remote/Staff-Engineer_R1"
        );
    }

    #[test]
    fn maps_ashby_fixture() {
        let data = json!({
            "jobs": [{
                "id": "job_123",
                "title": " Product Designer ",
                "department": "Design",
                "employmentType": "FullTime",
                "location": "New York",
                "secondaryLocations": [{"location": "Remote - US"}, {"location": "New York"}],
                "isRemote": true,
                "applyUrl": "https://jobs.ashbyhq.com/example/job_123/application",
                "publishedAt": "2026-10-01T09:00:00Z"
            }]
        });
        let identity = identity(AtsProvider::Ashby);
        let fetched = to_fetched_jobs(
            identity.clone(),
            map_ashby_jobs(&identity, "fixture", &data).unwrap(),
        );
        let job = &fetched[0].posting;
        assert_eq!(job.source, "ats/ashby/example");
        assert_eq!(job.source_job_id, "job_123");
        assert_eq!(job.title, "Product Designer");
        assert_eq!(job.location.as_deref(), Some("New York; Remote - US"));
        assert_eq!(job.workplace_type.as_deref(), Some("remote"));
        assert_eq!(job.department.as_deref(), Some("Design"));
        assert_eq!(job.employment_type.as_deref(), Some("FullTime"));
    }

    #[test]
    fn maps_smartrecruiters_fixture() {
        let data = json!({
            "content": [{
                "id": "743999999999999",
                "name": "Data Engineer",
                "company": {"identifier": "ExampleCo"},
                "department": {"label": "Data"},
                "typeOfEmployment": {"label": "Full-time"},
                "releasedDate": "2026-09-30T12:00:00.000Z",
                "location": {"fullLocation": "Berlin, Germany", "remote": false}
            }]
        });
        let identity = identity(AtsProvider::SmartRecruiters);
        let fetched = to_fetched_jobs(
            identity.clone(),
            map_smartrecruiters_jobs(&identity, "fixture", &data, "fallback").unwrap(),
        );
        let job = &fetched[0].posting;
        assert_eq!(job.source, "ats/smartrecruiters/example");
        assert_eq!(job.source_job_id, "743999999999999");
        assert_eq!(job.workplace_type.as_deref(), Some("onsite"));
        assert_eq!(job.location.as_deref(), Some("Berlin, Germany"));
        assert_eq!(
            job.url,
            "https://jobs.smartrecruiters.com/ExampleCo/743999999999999"
        );
    }

    #[test]
    fn maps_workable_fixture() {
        let data = json!({
            "jobs": [{
                "shortcode": "ABC123",
                "title": "Engineering Manager",
                "department": "Engineering",
                "city": "Paris",
                "state": "Ile-de-France",
                "country": "France",
                "telecommuting": true,
                "employment_type": "Full-time",
                "application_url": "https://apply.workable.com/example/j/ABC123/apply/",
                "published_on": "2026-09-29"
            }]
        });
        let identity = identity(AtsProvider::Workable);
        let fetched = to_fetched_jobs(
            identity.clone(),
            map_workable_jobs(&identity, "fixture", &data).unwrap(),
        );
        let job = &fetched[0].posting;
        assert_eq!(job.source, "ats/workable/example");
        assert_eq!(job.source_job_id, "ABC123");
        assert_eq!(
            job.location.as_deref(),
            Some("Paris; Ile-de-France; France")
        );
        assert_eq!(job.workplace_type.as_deref(), Some("remote"));
        assert_eq!(job.employment_type.as_deref(), Some("Full-time"));
    }

    #[test]
    fn maps_recruitee_fixture() {
        let data = json!({
            "offers": [{
                "id": 987,
                "title": "Security Engineer",
                "department": "Security",
                "employment_type_code": "full_time",
                "careers_url": "https://example.recruitee.com/o/security-engineer",
                "published_at": "2026-09-28T08:00:00Z",
                "remote": false,
                "hybrid": true,
                "locations": [{"name": "Amsterdam"}, {"name": "Amsterdam"}]
            }]
        });
        let identity = identity(AtsProvider::Recruitee);
        let fetched = to_fetched_jobs(
            identity.clone(),
            map_recruitee_jobs(&identity, "fixture", &data).unwrap(),
        );
        let job = &fetched[0].posting;
        assert_eq!(job.source, "ats/recruitee/example");
        assert_eq!(job.source_job_id, "987");
        assert_eq!(job.location.as_deref(), Some("Amsterdam"));
        assert_eq!(job.workplace_type.as_deref(), Some("hybrid"));
        assert_eq!(job.department.as_deref(), Some("Security"));
    }

    #[test]
    fn maps_bamboohr_fixture() {
        let data = json!({
            "result": [{
                "id": 55,
                "jobOpeningName": " Support Specialist ",
                "departmentLabel": "Customer Success",
                "employmentStatusLabel": "Contractor",
                "isRemote": false,
                "location": {"city": "Austin", "state": "TX"},
                "atsLocation": {"city": "Austin", "state": "TX", "country": "US"}
            }]
        });
        let identity = identity(AtsProvider::BambooHr);
        let fetched = to_fetched_jobs(
            identity.clone(),
            map_bamboohr_jobs(&identity, "fixture", &data, "example").unwrap(),
        );
        let job = &fetched[0].posting;
        assert_eq!(job.source, "ats/bamboohr/example");
        assert_eq!(job.source_job_id, "55");
        assert_eq!(job.title, "Support Specialist");
        assert_eq!(job.location.as_deref(), Some("Austin; TX; US"));
        assert_eq!(job.workplace_type.as_deref(), Some("onsite"));
        assert_eq!(job.url, "https://example.bamboohr.com/careers/55");
    }

    #[test]
    fn rejects_malformed_feed_instead_of_empty_success() {
        let identity = identity(AtsProvider::Lever);
        assert!(map_lever_jobs(&identity, "fixture", &json!({"jobs": []})).is_err());
        assert!(map_greenhouse_jobs(&identity, "fixture", &json!({})).is_err());
        assert!(map_workday_jobs(
            &identity,
            "fixture",
            &json!({"jobs": []}),
            &WorkdayConfig {
                tenant: "example".to_owned(),
                site: "External".to_owned(),
                shard: "wd1".to_owned(),
            }
        )
        .is_err());
    }

    #[test]
    fn safe_pagination_limits_are_capped() {
        assert_eq!(next_page_limit(100, 0, 500), Some(100));
        assert_eq!(next_page_limit(100, 450, 500), Some(50));
        assert_eq!(next_page_limit(100, 500, 500), None);
        assert_eq!(next_page_limit(20, 495, 500), Some(5));
        assert_eq!(
            AtsFetchOptions {
                max_jobs_per_company: 900
            }
            .capped_max_jobs(),
            900
        );
    }

    #[test]
    fn http_client_builders_configure_timeouts() {
        http_client_with_timeout(Duration::from_millis(50)).expect("custom timeout client builds");
    }

    #[test]
    fn provider_parser_accepts_common_aliases() {
        assert_eq!(
            "smart-recruiters".parse::<AtsProvider>().unwrap(),
            AtsProvider::SmartRecruiters
        );
        assert_eq!(
            "bamboo-hr".parse::<AtsProvider>().unwrap(),
            AtsProvider::BambooHr
        );
        assert!("unknown".parse::<AtsProvider>().is_err());
    }

    #[test]
    fn source_identity_uses_provider_and_slug() {
        let source = AtsCompanySource {
            company_id: Some("acme".to_owned()),
            company_name: "Acme".to_owned(),
            provider: AtsProvider::Ashby,
            slug: Some("acme-careers".to_owned()),
            workday: None,
        };
        assert_eq!(
            source.identity().unwrap().source_key(),
            "ats/ashby/acme-careers"
        );
    }
}
