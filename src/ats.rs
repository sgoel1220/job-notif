use std::{fmt, str::FromStr, time::Duration};

use serde::{Deserialize, Serialize};

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

mod ashby;
mod bamboohr;
mod client;
mod greenhouse;
mod helpers;
mod lever;
mod normalize;
mod personio;
mod recruitee;
mod smartrecruiters;
mod workable;
mod workday;

use ashby::*;
use bamboohr::*;
pub(crate) use client::{fetch_company_jobs, http_client_with_timeout};
use client::{get_json, get_text, post_json};
use greenhouse::*;
use helpers::*;
use lever::*;
use normalize::*;
use personio::*;
use recruitee::*;
use smartrecruiters::*;
use workable::*;
use workday::*;

#[cfg(test)]
use serde_json::json;

#[cfg(test)]
#[path = "ats/tests.rs"]
mod tests;
