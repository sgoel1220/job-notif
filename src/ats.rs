use std::{borrow::Cow, fmt, str::FromStr, time::Duration};

use quick_xml::{events::Event, Reader};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::job_postings::JobPosting;

pub(crate) const ATS_SOURCE_PREFIX: &str = "ats/";
const USER_AGENT: &str = "JobNotifier/0.1 (+https://github.com/ConorsCode/open-jobs-data port)";
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 20;
const DEFAULT_PAGE_SIZE: usize = 100;
const WORKDAY_PAGE_SIZE: usize = 20;
const MAX_JOBS_PER_COMPANY: usize = 500;

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

pub(crate) fn default_http_client() -> Result<reqwest::Client, AtsFetchError> {
    http_client_with_timeout(Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS))
}

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
        "https://api.ashbyhq.com/posting-api/job-board/{}",
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
    let slug = company.slug()?;
    let mut offset = 0usize;
    let mut total_found = usize::MAX;
    let mut jobs = Vec::<Value>::new();
    while offset < total_found {
        let Some(limit) = next_page_limit(DEFAULT_PAGE_SIZE, jobs.len(), options.capped_max_jobs())
        else {
            break;
        };
        let url = format!(
            "https://api.smartrecruiters.com/v1/companies/{}/postings?limit={limit}&offset={offset}",
            percent_encode_component(slug)
        );
        let data = get_json(client, identity, &url).await?;
        total_found = optional_usize(&data, "totalFound").unwrap_or(0);
        let page = array_at(&data, &["content"]).ok_or_else(|| {
            invalid_feed(
                identity,
                &url,
                "missing content array in SmartRecruiters page",
            )
        })?;
        if page.is_empty() {
            break;
        }
        let page_len = page.len();
        jobs.extend(page.iter().take(limit).cloned());
        offset += page_len;
    }
    let data = json!({ "content": jobs });
    map_smartrecruiters_jobs(
        identity,
        "smartrecruiters:aggregated",
        &data,
        company.slug()?,
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
    let mut total = usize::MAX;
    let mut postings = Vec::<Value>::new();
    while offset < total {
        let Some(limit) =
            next_page_limit(WORKDAY_PAGE_SIZE, postings.len(), options.capped_max_jobs())
        else {
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
        if offset == 0 {
            total = optional_usize(&data, "total").unwrap_or(0);
        }
        let page = array_at(&data, &["jobPostings"]).ok_or_else(|| {
            invalid_feed(identity, &url, "missing jobPostings array in Workday page")
        })?;
        if page.is_empty() {
            break;
        }
        let page_len = page.len();
        postings.extend(page.iter().take(limit).cloned());
        offset += page_len;
    }
    let data = json!({ "jobPostings": postings });
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
            let locations = string_at(job, &["location", "name"])
                .into_iter()
                .collect::<Vec<_>>();
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
            let explicit_remote = match string_at(job, &["workplaceType"]).as_deref() {
                Some("remote") => Some(true),
                Some("on-site") => Some(false),
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
                workplace_type: workplace_type(&locations, explicit_remote, false),
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
                workplace_type: workplace_type(&locations, bool_at(job, &["isRemote"]), false),
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
                    false,
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
    if positions.is_empty() && !xml.contains("<position") {
        return Err(invalid_feed(
            identity,
            url,
            "Personio XML contains no positions",
        ));
    }
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
            let locations = string_at(posting, &["locationsText"])
                .into_iter()
                .collect::<Vec<_>>();
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
                posted_at: None,
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
            let location = join_locations(&job.locations);
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
            posting.location = location.or(posting.location);
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
    reader.config_mut().trim_text(true);
    let mut positions = Vec::<PersonioPosition>::new();
    let mut current: Option<PersonioPosition> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) if element.name().as_ref() == b"position" => {
                current = Some(PersonioPosition::default());
            }
            Ok(Event::Start(element)) => {
                let tag = element.name().as_ref().to_vec();
                if let Some(position) = current.as_mut() {
                    let text = reader
                        .read_text(element.name())
                        .map_err(|error| invalid_feed(identity, url, error.to_string()))?;
                    let text = xml_text_to_string(text);
                    if text.is_empty() {
                        continue;
                    }
                    match tag.as_slice() {
                        b"id" => position.id = text,
                        b"name" => position.name = text,
                        b"department" => position.department = Some(text),
                        b"employmentType" => position.employment_type = Some(text),
                        b"createdAt" => position.created_at = Some(text),
                        b"office" => position.locations.extend(
                            text.split(',')
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                                .map(str::to_owned),
                        ),
                        _ => {}
                    }
                }
            }
            Ok(Event::End(element)) if element.name().as_ref() == b"position" => {
                if let Some(mut position) = current.take() {
                    dedupe(&mut position.locations);
                    if !position.id.trim().is_empty() && !position.name.trim().is_empty() {
                        positions.push(position);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => return Err(invalid_feed(identity, url, error.to_string())),
        }
    }
    Ok(positions)
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

fn next_page_limit(page_size: usize, current_jobs: usize, max_jobs: usize) -> Option<usize> {
    if current_jobs >= max_jobs || page_size == 0 || max_jobs == 0 {
        return None;
    }
    Some(page_size.min(max_jobs - current_jobs))
}

fn xml_text_to_string(text: Cow<'_, str>) -> String {
    let trimmed = text.trim();
    quick_xml::escape::unescape(trimmed)
        .map(Cow::into_owned)
        .unwrap_or_else(|_| trimmed.to_owned())
}

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
            500
        );
    }

    #[test]
    fn http_client_builders_configure_timeouts() {
        default_http_client().expect("default client builds");
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
