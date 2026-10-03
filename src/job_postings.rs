mod api;
mod filters;
mod locations;
mod normalization;
mod persistence;

pub(crate) use api::list;
pub(crate) use persistence::replace_source_snapshot;

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// Provider-independent job record. Provider-specific fields remain available in `details_json`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct JobPosting {
    pub(crate) source: String,
    pub(crate) source_job_id: String,
    pub(crate) title: String,
    pub(crate) company: String,
    pub(crate) location: Option<String>,
    pub(crate) workplace_type: Option<String>,
    pub(crate) employment_type: Option<String>,
    pub(crate) department: Option<String>,
    pub(crate) team: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) description_text: Option<String>,
    pub(crate) posted_at: Option<String>,
    pub(crate) salary_min: Option<f64>,
    pub(crate) salary_max: Option<f64>,
    pub(crate) salary_currency: Option<String>,
    pub(crate) salary_interval: Option<String>,
    pub(crate) url: String,
    pub(crate) details_json: String,
}

#[derive(Serialize, FromRow)]
pub(crate) struct StoredJobPosting {
    pub(crate) id: i64,
    pub(crate) title: String,
    pub(crate) company: String,
    pub(crate) location: Option<String>,
    pub(crate) workplace_type: Option<String>,
    pub(crate) employment_type: Option<String>,
    pub(crate) department: Option<String>,
    pub(crate) team: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) description_text: Option<String>,
    pub(crate) posted_at: Option<String>,
    pub(crate) salary_min: Option<f64>,
    pub(crate) salary_max: Option<f64>,
    pub(crate) salary_currency: Option<String>,
    pub(crate) salary_interval: Option<String>,
    pub(crate) url: String,
    pub(crate) is_active: bool,
    pub(crate) role_category: String,
    pub(crate) country_codes: Vec<String>,
    pub(crate) is_software_engineering: bool,
}

#[derive(Clone, Deserialize)]
pub(crate) struct ListingQuery {
    page: Option<u64>,
    page_size: Option<u64>,
    role_category: Option<String>,
    country: Option<String>,
    role: Option<String>,
    location: Option<String>,
    workplace: Option<String>,
    include_global: Option<bool>,
    include_unknown_workplace: Option<bool>,
    employment: Option<String>,
    department: Option<String>,
    posted_after: Option<String>,
    posted_before: Option<String>,
    salary_min: Option<f64>,
    salary_max: Option<f64>,
    currency: Option<String>,
    salary_interval: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct ListingPage {
    jobs: Vec<StoredJobPosting>,
    total: i64,
    page: u64,
    page_size: u64,
}

pub(crate) const DEFAULT_PAGE_SIZE: u64 = 25;
pub(crate) const MAX_PAGE_SIZE: u64 = 100;

#[cfg(test)]
use filters::{append_listing_filters, location_search_patterns, validate_salary_filters};
#[cfg(test)]
use persistence::location_for_storage;
#[cfg(test)]
use sqlx::{PgPool, Postgres, QueryBuilder};

#[cfg(test)]
#[path = "job_postings/tests.rs"]
mod tests;
