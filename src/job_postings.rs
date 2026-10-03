use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool, Postgres, QueryBuilder};

use crate::{errors::ApiError, AppState};

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

impl JobPosting {
    pub(crate) fn basic(
        source: &str,
        id: impl ToString,
        title: String,
        company: &str,
        location: Option<String>,
        workplace_type: Option<String>,
        url: String,
        details: impl Serialize,
    ) -> Self {
        let details_value = serde_json::to_value(&details).unwrap_or(serde_json::Value::Null);
        let details_json = serde_json::to_string(&details_value).unwrap_or_else(|_| "{}".into());
        let mut job = Self {
            source: source.into(),
            source_job_id: id.to_string(),
            title,
            company: company.into(),
            location,
            workplace_type,
            employment_type: None,
            department: None,
            team: None,
            description: None,
            description_text: None,
            posted_at: None,
            salary_min: None,
            salary_max: None,
            salary_currency: None,
            salary_interval: None,
            url,
            details_json,
        };
        // Normalize common ATS field names for filtering while retaining the complete original
        // object in details_json, including provider-specific and currently unknown fields.
        job.posted_at = first_date(
            &details_value,
            &[
                "postedAt",
                "posted_at",
                "publishedAt",
                "published_at",
                "first_published",
                "datePosted",
                "date_posted",
                "createdAt",
                "created_at",
            ],
        )
        .or(job.posted_at);
        job.employment_type = normalize_employment(
            first_string(
                &details_value,
                &["employmentType", "employment_type", "employmentStatus"],
            )
            .or_else(|| nested_string(&details_value, &["categories", "commitment"])),
        );
        job.department = first_string(&details_value, &["department", "departmentName"])
            .or_else(|| nested_string(&details_value, &["categories", "department"]))
            .or_else(|| named_array_values(&details_value, "departments"));
        job.team = first_string(&details_value, &["team", "teamName"])
            .or_else(|| nested_string(&details_value, &["categories", "team"]));
        job.salary_min = first_number(
            &details_value,
            &["salaryMin", "salary_min", "minSalary", "min_salary"],
        )
        .or_else(|| nested_number(&details_value, &["salaryRange", "min"]));
        job.salary_max = first_number(
            &details_value,
            &["salaryMax", "salary_max", "maxSalary", "max_salary"],
        )
        .or_else(|| nested_number(&details_value, &["salaryRange", "max"]));
        job.salary_currency = first_string(
            &details_value,
            &["salaryCurrency", "salary_currency", "currency"],
        )
        .or_else(|| nested_string(&details_value, &["salaryRange", "currency"]));
        job.salary_interval = first_string(
            &details_value,
            &[
                "salaryInterval",
                "salary_interval",
                "payPeriod",
                "salaryPeriod",
            ],
        )
        .or_else(|| nested_string(&details_value, &["salaryRange", "interval"]))
        .as_deref()
        .and_then(canonical_salary_interval);
        if let Some((minimum, maximum, currency, interval)) = ashby_salary(&details_value) {
            job.salary_min = job.salary_min.or(minimum);
            job.salary_max = job.salary_max.or(maximum);
            job.salary_currency = job.salary_currency.or(currency);
            job.salary_interval = job.salary_interval.or(interval);
        }
        job.description = first_string(
            &details_value,
            &[
                "description",
                "descriptionHtml",
                "descriptionBody",
                "content",
            ],
        );
        job.description_text = first_string(
            &details_value,
            &[
                "descriptionPlain",
                "descriptionBodyPlain",
                "openingPlain",
                "additionalPlain",
            ],
        );
        job.location = merge_locations(job.location, &details_value);
        job.workplace_type = normalize_workplace(job.workplace_type, &details_value);
        job
    }
}

fn first_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(value_to_string))
}

fn value_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn first_date(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let date = first_string(value, keys)?;
    date.parse::<i64>()
        .ok()
        .and_then(unix_millis_to_iso)
        .or(Some(date))
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

fn nested_string(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    value_to_string(current)
}

fn nested_number(value: &serde_json::Value, path: &[&str]) -> Option<f64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    match current {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.replace(',', "").parse().ok(),
        _ => None,
    }
}

fn named_array_values(value: &serde_json::Value, key: &str) -> Option<String> {
    let names: Vec<_> = value
        .get(key)?
        .as_array()?
        .iter()
        .filter_map(|item| item.get("name").and_then(value_to_string))
        .collect();
    (!names.is_empty()).then(|| names.join("; "))
}

fn merge_locations(location: Option<String>, value: &serde_json::Value) -> Option<String> {
    let mut all = Vec::<String>::new();
    if let Some(location) = location {
        all.push(location);
    } else if let Some(primary) = nested_string(value, &["categories", "location"]) {
        all.push(primary);
    }
    if let Some(primary_address) = value.pointer("/address/postalAddress") {
        if let Some(text) = location_item_text(primary_address) {
            all.push(text);
        }
    }
    for key in ["locations", "secondaryLocations", "offices"] {
        if let Some(items) = value.get(key).and_then(serde_json::Value::as_array) {
            for item in items {
                let text = location_item_text(item);
                if let Some(text) = text {
                    all.push(text);
                }
            }
        }
    }
    if let Some(primary) = nested_string(value, &["categories", "location"]) {
        all.push(primary);
    }
    if let Some(items) = value
        .pointer("/categories/allLocations")
        .and_then(serde_json::Value::as_array)
    {
        all.extend(items.iter().filter_map(value_to_string));
    }
    let mut seen = std::collections::HashSet::new();
    all.retain(|item| seen.insert(item.to_lowercase()));
    let complete = all.clone();
    all.retain(|item| {
        !complete.iter().any(|other| {
            !other.eq_ignore_ascii_case(item)
                && other
                    .split([',', ';'])
                    .any(|part| part.trim().eq_ignore_ascii_case(item.trim()))
        })
    });
    (!all.is_empty()).then(|| all.join("; "))
}

fn location_item_text(item: &serde_json::Value) -> Option<String> {
    if let Some(text) = value_to_string(item) {
        return Some(text);
    }
    let mut parts = Vec::new();
    'paths: for path in [
        &["name"][..],
        &["location"][..],
        &["addressLocality"][..],
        &["addressRegion"][..],
        &["addressCountry"][..],
        &["address", "postalAddress"][..],
        &["address", "postalAddress", "addressLocality"][..],
        &["address", "postalAddress", "addressRegion"][..],
        &["address", "postalAddress", "addressCountry", "name"][..],
        &["address", "postalAddress", "addressCountry"][..],
        &["address", "addressLocality"][..],
        &["address", "addressRegion"][..],
        &["address", "addressCountry", "name"][..],
        &["address", "addressCountry"][..],
        &["addressCountry", "name"][..],
        &["addressCountry"][..],
        &["country"][..],
        &["city"][..],
    ] {
        let mut current = item;
        for key in path {
            current = match current.get(*key) {
                Some(v) => v,
                None => continue 'paths,
            };
        }
        if let Some(text) = value_to_string(current) {
            parts.push(text);
        }
    }
    let mut seen = std::collections::HashSet::new();
    parts.retain(|part| seen.insert(part.to_lowercase()));
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn is_india_search(term: &str) -> bool {
    ["india", "bharat"]
        .iter()
        .any(|alias| term.eq_ignore_ascii_case(alias))
}

fn like_pattern(term: &str) -> String {
    format!(
        "%{}%",
        term.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

fn location_search_patterns(term: &str) -> Vec<String> {
    let mut terms = vec![like_pattern(term)];
    if is_india_search(term) {
        terms.extend(
            [
                "Bengaluru",
                "Bangalore",
                "Mumbai",
                "Delhi",
                "New Delhi",
                "Hyderabad",
                "Pune",
                "Chennai",
                "Kolkata",
                "Noida",
                "Gurugram",
                "Gurgaon",
                "Ahmedabad",
                "Jaipur",
            ]
            .into_iter()
            .map(like_pattern),
        );
    }
    terms
}

/// Ashby's public feed exposes structured salary values in the source's stated currency and
/// interval (for example USD / "1 YEAR"). We preserve the numeric amounts as supplied and do
/// not annualize, convert currencies, or treat equity/bonus components as salary.
fn canonical_salary_interval(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    let tokens: String = normalized
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    let digits = tokens.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digits > 0 && &tokens[..digits] != "1" {
        return None;
    }
    let unit = &tokens[digits..];
    match unit {
        "h" | "hr" | "hrs" | "hour" | "hours" | "hourly" | "perhour" => Some("hour".into()),
        "d" | "day" | "days" | "daily" | "perday" => Some("day".into()),
        "w" | "wk" | "wks" | "week" | "weeks" | "weekly" | "perweek" => Some("week".into()),
        "mo" | "mos" | "month" | "months" | "monthly" | "permonth" => Some("month".into()),
        "y" | "yr" | "yrs" | "year" | "years" | "yearly" | "annual" | "annually" | "peryear" => {
            Some("year".into())
        }
        _ => None,
    }
}

fn ashby_salary(
    value: &serde_json::Value,
) -> Option<(Option<f64>, Option<f64>, Option<String>, Option<String>)> {
    let compensation = value.get("compensation")?;
    let components = compensation
        .get("summaryComponents")
        .and_then(serde_json::Value::as_array)
        .filter(|items| !items.is_empty())
        .or_else(|| {
            compensation
                .pointer("/compensationTiers/0/components")
                .and_then(serde_json::Value::as_array)
        })?;
    let salary = components.iter().find(|item| {
        item.get("compensationType")
            .and_then(value_to_string)
            .is_some_and(|kind| kind.eq_ignore_ascii_case("salary"))
    })?;
    let minimum = first_number(salary, &["minValue"]);
    let maximum = first_number(salary, &["maxValue"]);
    (minimum.is_some() || maximum.is_some()).then(|| {
        (
            minimum,
            maximum,
            first_string(salary, &["currencyCode"]),
            first_string(salary, &["interval"])
                .as_deref()
                .and_then(canonical_salary_interval),
        )
    })
}

fn normalize_employment(value: Option<String>) -> Option<String> {
    let value = value?;
    let normalized = value.to_ascii_lowercase();
    let compact: String = normalized
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    let canonical = match compact.as_str() {
        "fulltime" | "fulltimeemployee" | "fulltimeemployment" => "Full-time",
        "parttime" | "parttimeemployee" | "parttimeemployment" => "Part-time",
        "intern" | "internship" => "Internship",
        "contractor" | "contract" | "fixedterm" | "fixedtermcontract" => "Contract",
        "permanent" => "Permanent",
        "temporary" | "temp" => "Temporary",
        _ => return Some(value),
    };
    Some(canonical.to_owned())
}

fn employment_key(value: &str) -> String {
    normalize_employment(Some(value.to_owned()))
        .unwrap_or_default()
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect()
}

fn append_canonical_employment_filter<'a>(query: &mut QueryBuilder<'a, Postgres>, value: &str) {
    query.push(" AND CASE ");
    for (canonical, aliases) in [
        (
            "fulltime",
            "'fulltime', 'fulltimeemployee', 'fulltimeemployment'",
        ),
        (
            "parttime",
            "'parttime', 'parttimeemployee', 'parttimeemployment'",
        ),
        ("internship", "'intern', 'internship'"),
        (
            "contract",
            "'contract', 'contractor', 'fixedterm', 'fixedtermcontract'",
        ),
        ("permanent", "'permanent'"),
        ("temporary", "'temporary', 'temp'"),
    ] {
        query
            .push(" WHEN ")
            .push("regexp_replace(lower(COALESCE(employment_type, '')), '[^a-z0-9]', '', 'g') IN (")
            .push(aliases)
            .push(") THEN '")
            .push(canonical)
            .push("'");
    }
    query
        .push(" ELSE regexp_replace(lower(COALESCE(employment_type, '')), '[^a-z0-9]', '', 'g') END = ")
        .push_bind(employment_key(value));
}

fn normalize_workplace(current: Option<String>, value: &serde_json::Value) -> Option<String> {
    let label = first_string(value, &["workplaceType", "workplace_type", "workplace"])
        .or(current)
        .or_else(|| {
            value
                .get("isRemote")
                .or_else(|| value.get("is_remote"))
                .and_then(serde_json::Value::as_bool)
                .filter(|remote| *remote)
                .map(|_| "remote".to_owned())
        })?;
    let lower = label.to_lowercase();
    if lower.contains("remote") {
        Some("remote".into())
    } else if lower.contains("hybrid") {
        Some("hybrid".into())
    } else if ["on-site", "onsite", "in-office", "office"]
        .iter()
        .any(|word| lower.contains(word))
    {
        Some("onsite".into())
    } else {
        Some(label)
    }
}

fn first_number(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|value| match value {
            serde_json::Value::Number(number) => number.as_f64(),
            serde_json::Value::String(text) => text.replace(',', "").parse().ok(),
            _ => None,
        })
    })
}

fn location_for_storage(job: &JobPosting) -> Option<String> {
    let details: serde_json::Value = serde_json::from_str(&job.details_json).ok()?;
    merge_locations(job.location.clone(), &details)
}

/// Reconcile one company's complete snapshot for one exact ATS source.
///
/// The exact source equality avoids prefix collisions such as `lever` matching
/// `lever-v2`. Validation runs before the transaction so a malformed snapshot
/// cannot deactivate existing rows for the source.
pub(crate) async fn replace_source_snapshot(
    db: &PgPool,
    company: &str,
    source: &str,
    jobs: &[JobPosting],
) -> Result<(), ApiError> {
    if jobs
        .iter()
        .any(|job| job.company != company || job.source != source)
    {
        return Err(ApiError::bad_request(
            "Snapshot contains jobs outside the requested company/source.",
        ));
    }

    let mut tx = db.begin().await.map_err(ApiError::database)?;
    sqlx::query("UPDATE job_postings SET is_active = FALSE WHERE company = $1 AND source = $2")
        .bind(company)
        .bind(source)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    for job in jobs {
        sqlx::query(
            "INSERT INTO job_postings (
                source, source_job_id, title, company, location, workplace_type, employment_type,
                department, team, description, description_text, posted_at, salary_min, salary_max,
                salary_currency, salary_interval, url, details_json, is_active
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, TRUE)
             ON CONFLICT(company, source, source_job_id) DO UPDATE SET
                title = excluded.title, location = excluded.location,
                workplace_type = excluded.workplace_type, employment_type = excluded.employment_type,
                department = excluded.department, team = excluded.team,
                description = excluded.description, description_text = excluded.description_text,
                posted_at = excluded.posted_at, salary_min = excluded.salary_min,
                salary_max = excluded.salary_max, salary_currency = excluded.salary_currency,
                salary_interval = excluded.salary_interval,
                url = excluded.url, details_json = excluded.details_json,
                last_seen_at = CURRENT_TIMESTAMP::text, is_active = TRUE"
        )
        .bind(&job.source).bind(&job.source_job_id).bind(&job.title).bind(&job.company)
        .bind(location_for_storage(job)).bind(&job.workplace_type).bind(normalize_employment(job.employment_type.clone()))
        .bind(&job.department).bind(&job.team).bind(&job.description).bind(&job.description_text)
        .bind(&job.posted_at).bind(job.salary_min).bind(job.salary_max).bind(&job.salary_currency)
        .bind(job.salary_interval.as_deref().and_then(canonical_salary_interval)).bind(&job.url).bind(&job.details_json)
        .execute(&mut *tx).await.map_err(ApiError::database)?;
    }
    tx.commit().await.map_err(ApiError::database)
}

/// Reconcile a complete set of namespaced source rows. Unrelated providers are never deactivated.
pub(crate) async fn replace_source_prefix_snapshot(
    db: &PgPool,
    source_prefix: &str,
    jobs: &[JobPosting],
) -> Result<(), ApiError> {
    let mut tx = db.begin().await.map_err(ApiError::database)?;
    sqlx::query("UPDATE job_postings SET is_active = FALSE WHERE source LIKE $1 || '%'")
        .bind(source_prefix)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    for job in jobs {
        sqlx::query(
            "INSERT INTO job_postings (
                source, source_job_id, title, company, location, workplace_type, employment_type,
                department, team, description, description_text, posted_at, salary_min, salary_max,
                salary_currency, salary_interval, url, details_json, is_active
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, TRUE)
             ON CONFLICT(company, source, source_job_id) DO UPDATE SET
                title = excluded.title, location = excluded.location,
                workplace_type = excluded.workplace_type, employment_type = excluded.employment_type,
                department = excluded.department, team = excluded.team,
                description = excluded.description, description_text = excluded.description_text,
                posted_at = excluded.posted_at, salary_min = excluded.salary_min,
                salary_max = excluded.salary_max, salary_currency = excluded.salary_currency,
                salary_interval = excluded.salary_interval,
                url = excluded.url, details_json = excluded.details_json,
                last_seen_at = CURRENT_TIMESTAMP::text, is_active = TRUE",
        )
        .bind(&job.source)
        .bind(&job.source_job_id)
        .bind(&job.title)
        .bind(&job.company)
        .bind(location_for_storage(job))
        .bind(&job.workplace_type)
        .bind(normalize_employment(job.employment_type.clone()))
        .bind(&job.department)
        .bind(&job.team)
        .bind(&job.description)
        .bind(&job.description_text)
        .bind(&job.posted_at)
        .bind(job.salary_min)
        .bind(job.salary_max)
        .bind(&job.salary_currency)
        .bind(job.salary_interval.as_deref().and_then(canonical_salary_interval))
        .bind(&job.url)
        .bind(&job.details_json)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    }
    tx.commit().await.map_err(ApiError::database)
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

fn append_listing_filters<'a>(query: &mut QueryBuilder<'a, Postgres>, filters: &'a ListingQuery) {
    query.push(" WHERE is_active = TRUE");
    if let Some(category) = filters.role_category.as_deref().filter(|v| !v.is_empty()) {
        if category == "software-engineering" {
            query.push(" AND is_software_engineering = TRUE");
        } else {
            query.push(" AND role_category = ").push_bind(category);
            if category == "other" {
                query.push(" AND is_software_engineering = FALSE");
            }
        }
    }
    if let Some(country) = filters.country.as_deref().filter(|v| !v.is_empty()) {
        query
            .push(" AND (country_codes @> ")
            .push_bind(vec![country.to_owned()]);
        if filters.include_global.unwrap_or(false) {
            query.push(" OR EXISTS (SELECT 1 FROM unnest(string_to_array(COALESCE(location, ''), ';')) AS location_part(value) WHERE LOWER(BTRIM(location_part.value)) IN ('remote', 'worldwide', 'anywhere', 'global'))");
        }
        query.push(")");
    }
    if let Some(value) = filters
        .role
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        // Normalize the multi-word alias before tokenizing so "front end" stays one concept.
        let normalized_role = value
            .to_ascii_lowercase()
            .replace("front-end", "frontend")
            .replace("front end", "frontend");
        for token in normalized_role.split_whitespace() {
            let normalized = token.trim_matches(|ch: char| {
                !ch.is_alphanumeric() && !['-', '%', '_', '\\'].contains(&ch)
            });
            if normalized.is_empty() {
                continue;
            }
            if matches!(
                normalized,
                "intern" | "interns" | "internship" | "internships"
            ) {
                // An explicit word family, not a prefix: "intern*" also matches
                // "internal", "international", and "internet".
                query
                    .push(" AND title ~* ")
                    .push_bind(r"\m(intern|interns|internship|internships)\M");
            } else if normalized == "frontend" {
                query.push(" AND (title ILIKE '%frontend%' OR title ILIKE '%front-end%' OR title ILIKE '%front end%')");
            } else {
                query
                    .push(" AND title ILIKE ")
                    .push_bind(like_pattern(normalized))
                    .push(" ESCAPE E'\\\\'");
            }
        }
    }
    if let Some(value) = filters
        .location
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let term = value.trim();
        let patterns = location_search_patterns(term);
        query.push(" AND (");
        for (index, pattern) in patterns.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("COALESCE(location, '') ILIKE ")
                .push_bind(pattern.clone())
                .push(" ESCAPE E'\\\\'");
        }
        if filters.include_global.unwrap_or(false) {
            query.push(" OR EXISTS (SELECT 1 FROM unnest(string_to_array(COALESCE(location, ''), ';')) AS location_part(value) WHERE LOWER(BTRIM(location_part.value)) IN ('remote', 'worldwide', 'anywhere', 'global'))");
        }
        query.push(")");
    }
    if let Some(value) = filters
        .workplace
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        if value.eq_ignore_ascii_case("unknown") {
            query.push(" AND (workplace_type IS NULL OR BTRIM(workplace_type) = '')");
        } else if filters.include_unknown_workplace.unwrap_or(false) {
            query
                .push(" AND (COALESCE(workplace_type, '') ILIKE ")
                .push_bind(like_pattern(value.trim()))
                .push(" ESCAPE E'\\\\' OR workplace_type IS NULL OR BTRIM(workplace_type) = '')");
        } else {
            query
                .push(" AND COALESCE(workplace_type, '') ILIKE ")
                .push_bind(like_pattern(value.trim()))
                .push(" ESCAPE E'\\\\'");
        }
    }
    if let Some(value) = filters
        .employment
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        append_canonical_employment_filter(query, value.trim());
    }
    if let Some(value) = filters
        .department
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let pattern = like_pattern(value.trim());
        query
            .push(" AND (COALESCE(department, '') ILIKE ")
            .push_bind(pattern.clone())
            .push(" ESCAPE E'\\\\' OR COALESCE(team, '') ILIKE ")
            .push_bind(pattern)
            .push(" ESCAPE E'\\\\')");
    }
    if let Some(value) = filters
        .posted_after
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        query
            .push(" AND LEFT(posted_at, 10) >= ")
            .push_bind(value.to_owned());
    }
    if let Some(value) = filters
        .posted_before
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        query
            .push(" AND LEFT(posted_at, 10) <= ")
            .push_bind(value.to_owned());
    }
    if filters.salary_min.is_some() || filters.salary_max.is_some() {
        if let (Some(currency), Some(interval)) = (
            filters.currency.as_deref(),
            filters.salary_interval.as_deref(),
        ) {
            query
                .push(" AND LOWER(BTRIM(COALESCE(salary_currency, ''))) = ")
                .push_bind(currency.trim().to_ascii_lowercase());
            query
                .push(" AND salary_interval = ")
                .push_bind(canonical_salary_interval(interval).unwrap_or_default());
        }
    } else if let Some(interval) = filters.salary_interval.as_deref() {
        query
            .push(" AND salary_interval = ")
            .push_bind(canonical_salary_interval(interval).unwrap_or_default());
    }
    if let Some(value) = filters.salary_min {
        query
            .push(" AND COALESCE(salary_max, salary_min) >= ")
            .push_bind(value);
    }
    if let Some(value) = filters.salary_max {
        query
            .push(" AND COALESCE(salary_min, salary_max) <= ")
            .push_bind(value);
    }
    if let Some(value) = filters
        .currency
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        if filters.salary_min.is_some() || filters.salary_max.is_some() {
            // Numeric filters use the exact requested currency above.
        } else {
            query
                .push(" AND COALESCE(salary_currency, '') ILIKE ")
                .push_bind(like_pattern(value.trim()))
                .push(" ESCAPE E'\\\\'");
        }
    }
}

fn validate_salary_filters(filters: &ListingQuery) -> Result<(), ApiError> {
    if filters.role_category.as_deref().is_some_and(|value| {
        ![
            "",
            "intern",
            "sde-1",
            "sde-2",
            "software-engineering",
            "other",
        ]
        .contains(&value)
    }) {
        return Err(ApiError::bad_request(
            "Unsupported role_category; use intern, sde-1, sde-2, software-engineering, or other.",
        ));
    }
    if filters
        .country
        .as_deref()
        .is_some_and(|value| !["", "US", "IN"].contains(&value))
    {
        return Err(ApiError::bad_request("Unsupported country; use US or IN."));
    }
    let has_numeric = filters.salary_min.is_some() || filters.salary_max.is_some();
    let currency = filters
        .currency
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let interval = filters
        .salary_interval
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    if has_numeric && (currency.is_none() || interval.is_none()) {
        return Err(ApiError::bad_request(
            "Numeric salary filters require both currency and salary_interval.",
        ));
    }
    if has_numeric && currency.is_some_and(|value| value.chars().any(char::is_whitespace)) {
        return Err(ApiError::bad_request(
            "Currency must be a single currency code.",
        ));
    }
    if let Some(interval) = interval {
        if canonical_salary_interval(interval).is_none() {
            return Err(ApiError::bad_request(
                "Unsupported salary_interval; use hour, day, week, month, or year.",
            ));
        }
    }
    if filters.salary_min.is_some_and(|value| !value.is_finite())
        || filters.salary_max.is_some_and(|value| !value.is_finite())
    {
        return Err(ApiError::bad_request(
            "Salary bounds must be finite numbers.",
        ));
    }
    Ok(())
}

pub(crate) async fn list(
    State(state): State<AppState>,
    Query(filters): Query<ListingQuery>,
) -> Result<Json<ListingPage>, ApiError> {
    validate_salary_filters(&filters)?;
    let page = filters.page.unwrap_or(1).max(1);
    let page_size = filters
        .page_size
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);

    let mut count_query = QueryBuilder::<Postgres>::new("SELECT COUNT(*) FROM job_postings");
    append_listing_filters(&mut count_query, &filters);
    let total = count_query
        .build_query_scalar::<i64>()
        .fetch_one(&state.db)
        .await
        .map_err(ApiError::database)?;

    let offset = (page - 1).saturating_mul(page_size);
    let mut rows_query = QueryBuilder::<Postgres>::new(
        "SELECT id, title, company, location, workplace_type, employment_type,
         department, team, LEFT(description, 2000) AS description,
         LEFT(description_text, 2000) AS description_text, posted_at, salary_min,
         salary_max, salary_currency, salary_interval, url, is_active,
         role_category, country_codes, is_software_engineering FROM job_postings",
    );
    append_listing_filters(&mut rows_query, &filters);
    rows_query
        .push(" ORDER BY company, title, id LIMIT ")
        .push_bind(page_size as i64)
        .push(" OFFSET ")
        .push_bind(offset.min(i64::MAX as u64) as i64);
    let jobs = rows_query
        .build_query_as::<StoredJobPosting>()
        .fetch_all(&state.db)
        .await
        .map_err(ApiError::database)?;

    Ok(Json(ListingPage {
        jobs,
        total,
        page,
        page_size,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Execute;

    fn empty_filters() -> ListingQuery {
        ListingQuery {
            page: None,
            page_size: None,
            role_category: None,
            country: None,
            role: None,
            location: None,
            workplace: None,
            include_global: None,
            include_unknown_workplace: None,
            employment: None,
            department: None,
            posted_after: None,
            posted_before: None,
            salary_min: None,
            salary_max: None,
            currency: None,
            salary_interval: None,
        }
    }

    async fn matching_count(db: &PgPool, filters: &ListingQuery) -> i64 {
        let mut debug_query = QueryBuilder::<Postgres>::new("SELECT COUNT(*) FROM job_postings");
        append_listing_filters(&mut debug_query, filters);
        let sql = debug_query.build().sql().to_owned();
        let mut query = QueryBuilder::<Postgres>::new("SELECT COUNT(*) FROM job_postings");
        append_listing_filters(&mut query, filters);
        query
            .build_query_scalar::<i64>()
            .fetch_one(db)
            .await
            .unwrap_or_else(|error| panic!("{error}; SQL: {sql}"))
    }

    #[test]
    fn role_filter_searches_role_metadata_not_description_or_company() {
        let filters = ListingQuery {
            page: None,
            page_size: None,
            role_category: None,
            country: None,
            role: Some("intern".into()),
            location: None,
            workplace: None,
            include_global: None,
            include_unknown_workplace: None,
            employment: None,
            department: None,
            posted_after: None,
            posted_before: None,
            salary_min: None,
            salary_max: None,
            currency: None,
            salary_interval: None,
        };
        let mut query = QueryBuilder::<Postgres>::new("SELECT * FROM job_postings");
        append_listing_filters(&mut query, &filters);
        let sql = query.build().sql().to_owned();
        assert!(sql.contains("title ~*"));
        assert!(!sql.contains("department"));
        assert!(!sql.contains("team"));
        assert!(!sql.contains("description"));
        assert!(!sql.contains("company ILIKE"));
    }

    #[test]
    fn role_search_uses_all_title_tokens_and_frontend_aliases_only() {
        let filters = ListingQuery {
            page: None,
            page_size: None,
            role_category: None,
            country: None,
            role: Some("software engineer".into()),
            location: None,
            workplace: None,
            include_global: None,
            include_unknown_workplace: None,
            employment: None,
            department: None,
            posted_after: None,
            posted_before: None,
            salary_min: None,
            salary_max: None,
            currency: None,
            salary_interval: None,
        };
        let mut query = QueryBuilder::<Postgres>::new("SELECT * FROM job_postings");
        append_listing_filters(&mut query, &filters);
        let sql = query.build().sql().to_owned();
        assert_eq!(sql.matches("title ILIKE").count(), 2);
        assert!(!sql.contains("company ILIKE") && !sql.contains("description ILIKE"));

        let filters = ListingQuery {
            role: Some("front-end".into()),
            ..filters.clone()
        };
        let mut query = QueryBuilder::<Postgres>::new("SELECT * FROM job_postings");
        append_listing_filters(&mut query, &filters);
        let sql = query.build().sql().to_owned();
        assert_eq!(sql.matches("title ILIKE").count(), 3);
        assert!(sql.contains("OR title ILIKE"));
    }

    #[test]
    fn india_aliases_and_inclusive_filters_are_opt_in() {
        let filters = ListingQuery {
            page: None,
            page_size: None,
            role_category: None,
            country: None,
            role: None,
            location: Some("India".into()),
            workplace: Some("remote".into()),
            include_global: Some(true),
            include_unknown_workplace: Some(true),
            employment: None,
            department: None,
            posted_after: None,
            posted_before: None,
            salary_min: None,
            salary_max: None,
            currency: None,
            salary_interval: None,
        };
        let mut query = QueryBuilder::<Postgres>::new("SELECT * FROM job_postings");
        append_listing_filters(&mut query, &filters);
        let sql = query.build().sql().to_owned();
        assert!(sql.contains("worldwide") && sql.contains("anywhere"));
        let aliases = location_search_patterns("India");
        assert!(
            aliases.contains(&"%Bengaluru%".to_owned())
                && aliases.contains(&"%Bangalore%".to_owned())
        );
        assert!(sql.matches("COALESCE(location, '') ILIKE").count() >= aliases.len());
        assert!(sql.contains("workplace_type IS NULL"));

        let strict = ListingQuery {
            include_global: None,
            include_unknown_workplace: None,
            ..filters
        };
        let mut query = QueryBuilder::<Postgres>::new("SELECT * FROM job_postings");
        append_listing_filters(&mut query, &strict);
        let sql = query.build().sql().to_owned();
        assert!(!sql.contains("worldwide") && !sql.contains("workplace_type IS NULL"));
    }

    #[test]
    fn salary_filters_require_explicit_currency_and_supported_interval() {
        let mut query = empty_filters();
        query.salary_min = Some(100.0);
        assert!(validate_salary_filters(&query).is_err());
        query.currency = Some("USD".into());
        assert!(validate_salary_filters(&query).is_err());
        query.salary_interval = Some("decade".into());
        assert!(validate_salary_filters(&query).is_err());
        query.salary_interval = Some("1 YEAR".into());
        assert!(validate_salary_filters(&query).is_ok());
        query.salary_min = Some(f64::INFINITY);
        assert!(validate_salary_filters(&query).is_err());
    }

    #[test]
    fn category_filters_reject_unsupported_values() {
        for role in ["sde-3", "intern%", "Intern"] {
            let filters = ListingQuery {
                role_category: Some(role.into()),
                ..empty_filters()
            };
            assert!(validate_salary_filters(&filters).is_err());
        }
        for country in ["USA", "India", "us", "GB"] {
            let filters = ListingQuery {
                country: Some(country.into()),
                ..empty_filters()
            };
            assert!(validate_salary_filters(&filters).is_err());
        }
    }

    #[tokio::test]
    async fn category_migration_backfills_and_reclassifies_jobs_on_update() {
        let db = crate::test_database().await;
        // Recreate the pre-migration shape to verify backfill, not only new inserts.
        sqlx::raw_sql("ALTER TABLE job_postings DROP COLUMN role_category, DROP COLUMN country_codes; DROP FUNCTION job_role_category(TEXT, TEXT); DROP FUNCTION job_country_codes(TEXT);")
            .execute(&db).await.unwrap();
        sqlx::query("INSERT INTO job_postings (source, source_job_id, title, company, location, url) VALUES ('test', 'old', 'Software Engineer Intern', 'Example', 'Bengaluru', 'https://example.com')")
            .execute(&db).await.unwrap();
        sqlx::raw_sql(include_str!("../migrations/0004_job_categories.sql"))
            .execute(&db)
            .await
            .unwrap();
        let filters = ListingQuery {
            role_category: Some("intern".into()),
            country: Some("IN".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &filters).await, 1);
        sqlx::query("UPDATE job_postings SET title = 'Software Engineer II', location = 'United States' WHERE source_job_id = 'old'")
            .execute(&db).await.unwrap();
        assert_eq!(matching_count(&db, &filters).await, 0);
        let filters = ListingQuery {
            role_category: Some("sde-2".into()),
            country: Some("US".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &filters).await, 1);
    }

    #[tokio::test]
    async fn database_category_rules_are_conservative_and_exact() {
        let db = crate::test_database().await;
        let fixtures = [
            (
                "Software Engineer Intern",
                None,
                "India",
                "intern",
                vec!["IN"],
            ),
            ("Summer Internship", None, "US", "intern", vec!["US"]),
            (
                "Software Engineer",
                Some("Internship"),
                "USA",
                "intern",
                vec!["US"],
            ),
            ("Internal Auditor", None, "London", "other", vec![]),
            ("International Sales", None, "Worldwide", "other", vec![]),
            ("Internet Engineer", None, "Remote", "other", vec![]),
            (
                "Software Engineer I",
                None,
                "Bengaluru",
                "sde-1",
                vec!["IN"],
            ),
            (
                "Software Development Engineer 1",
                None,
                "New Delhi",
                "sde-1",
                vec!["IN"],
            ),
            ("SDE-1", None, "U.S.A.", "sde-1", vec!["US"]),
            ("SDE1", None, "San Francisco", "sde-1", vec!["US"]),
            (
                "Junior Software Developer",
                None,
                "Seattle",
                "sde-1",
                vec!["US"],
            ),
            (
                "Software Engineer, New Graduate",
                None,
                "Mumbai, India",
                "sde-1",
                vec!["IN"],
            ),
            (
                "Software Engineer II",
                None,
                "United States of America",
                "sde-2",
                vec!["US"],
            ),
            (
                "Software Development Engineer 2",
                None,
                "USA; India",
                "sde-2",
                vec!["US", "IN"],
            ),
            ("SDE2", None, "United States", "sde-2", vec!["US"]),
            ("SDE-2", None, "U.S.", "sde-2", vec!["US"]),
            (
                "Mid-level Software Engineer",
                None,
                "Hyderabad",
                "sde-2",
                vec!["IN"],
            ),
            (
                "Software Engineer, Level II",
                None,
                "Pune",
                "sde-2",
                vec!["IN"],
            ),
            (
                "Senior Software Engineer II",
                None,
                "Indiana, US",
                "other",
                vec!["US"],
            ),
            ("SDE-10", None, "Indianapolis", "other", vec![]),
            ("Software Engineer III", None, "Australia", "other", vec![]),
            ("Software Engineer", None, "Remote", "other", vec![]),
            ("Junior Accountant", None, "Canada", "other", vec![]),
        ];
        for (index, (title, employment, location, expected_role, expected_countries)) in
            fixtures.iter().enumerate()
        {
            let (role, countries): (String, Vec<String>) = sqlx::query_as("INSERT INTO job_postings (source, source_job_id, title, company, employment_type, location, url) VALUES ('test', $1, $2, 'Example', $3, $4, 'https://example.com') RETURNING role_category, country_codes")
                .bind(index.to_string()).bind(title).bind(employment).bind(location)
                .fetch_one(&db).await.unwrap();
            assert_eq!(role, *expected_role, "title={title}");
            assert_eq!(countries, *expected_countries, "location={location}");
        }
        for role in ["intern", "sde-1", "sde-2", "other"] {
            for country in [None, Some("US"), Some("IN")] {
                let expected = fixtures
                    .iter()
                    .filter(|(title, _, _, category, countries)| {
                        *category == role
                            && (role != "other"
                                || [
                                    "Internal Auditor",
                                    "International Sales",
                                    "Internet Engineer",
                                    "Junior Accountant",
                                ]
                                .contains(title))
                            && country.is_none_or(|c| countries.contains(&c))
                    })
                    .count() as i64;
                let filters = ListingQuery {
                    role_category: Some(role.into()),
                    country: country.map(str::to_owned),
                    ..empty_filters()
                };
                assert_eq!(
                    matching_count(&db, &filters).await,
                    expected,
                    "role={role}, country={country:?}"
                );
            }
        }
        let filters = ListingQuery {
            country: Some("IN".into()),
            ..empty_filters()
        };
        let strict = matching_count(&db, &filters).await;
        let inclusive = ListingQuery {
            include_global: Some(true),
            ..filters.clone()
        };
        assert_eq!(matching_count(&db, &inclusive).await, strict + 3);
        sqlx::query(
            "UPDATE job_postings SET is_active = FALSE WHERE title = 'Software Engineer Intern'",
        )
        .execute(&db)
        .await
        .unwrap();
        let filters = ListingQuery {
            role_category: Some("intern".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &filters).await, 2);
    }

    #[tokio::test]
    async fn software_engineering_filter_spans_levels_and_other_excludes_it() {
        let db = crate::test_database().await;
        let software_titles = [
            "Software Engineer Intern",
            "Software Engineer I",
            "Software Engineer II",
            "Senior Software Engineer",
            "Software Engineer",
            "Software Developer",
            "Software Engineering Manager",
            "SDE-2",
            "SDE10",
            "Frontend Engineer",
            "Back-end Developer",
            "Full Stack Engineer",
            "Mobile Engineer",
            "iOS Developer",
            "Android Software Engineer",
            "Embedded Software Engineer",
        ];
        let other_titles = [
            "Accountant",
            "Internal Auditor",
            "International Sales",
            "Internet Engineer",
            "Hardware Engineer",
            "Software Sales Manager",
            "Software EngineeringIntern",
        ];
        for (index, title) in software_titles
            .iter()
            .chain(other_titles.iter())
            .chain([&"Design Intern"])
            .enumerate()
        {
            sqlx::query("INSERT INTO job_postings (source, source_job_id, title, company, location, url) VALUES ('test', $1, $2, 'Example', 'India', 'https://example.com')")
                .bind(index.to_string()).bind(title).execute(&db).await.unwrap();
        }
        let filters = ListingQuery {
            role_category: Some("software-engineering".into()),
            country: Some("IN".into()),
            ..empty_filters()
        };
        assert!(validate_salary_filters(&filters).is_ok());
        assert_eq!(
            matching_count(&db, &filters).await,
            software_titles.len() as i64
        );
        let other = ListingQuery {
            role_category: Some("other".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &other).await, other_titles.len() as i64);
        let interns = ListingQuery {
            role_category: Some("intern".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &interns).await, 2);
        sqlx::query(
            "UPDATE job_postings SET title = 'Backend Engineer' WHERE title = 'Accountant'",
        )
        .execute(&db)
        .await
        .unwrap();
        assert_eq!(
            matching_count(&db, &filters).await,
            software_titles.len() as i64 + 1
        );
        assert_eq!(
            matching_count(&db, &other).await,
            other_titles.len() as i64 - 1
        );
    }

    #[tokio::test]
    async fn intern_search_matches_whole_word_family_not_unrelated_prefixes() {
        let db = crate::test_database().await;
        let titles = [
            "Software Engineer Intern",
            "INTERN — Product",
            "Summer Internship",
            "Engineering Internships",
            "Design Interns",
            "Software Engineer (Intern)",
            "Software Engineer - Intern/Co-op",
            "Internal Auditor",
            "International Sales",
            "Internet Engineer",
            "Software Engineer",
            "InternshipCoordinator",
            "Winterintern",
        ];
        for (index, title) in titles.iter().enumerate() {
            sqlx::query("INSERT INTO job_postings (source, source_job_id, title, company, url, is_active, description_text) VALUES ('test', $1, $2, 'Intern Company', 'https://example.com', TRUE, 'intern internship')")
                .bind(index.to_string())
                .bind(title)
                .execute(&db)
                .await
                .unwrap();
        }
        for role in ["intern", "INTERN", "interns", "internship", "internships"] {
            let filters = ListingQuery {
                role: Some(role.into()),
                ..empty_filters()
            };
            assert_eq!(matching_count(&db, &filters).await, 7, "role={role}");
        }
        let filters = ListingQuery {
            role: Some("software intern".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &filters).await, 3);
        let filters = ListingQuery {
            role: Some("internal".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &filters).await, 1);
        sqlx::query("UPDATE job_postings SET is_active = FALSE WHERE title = 'Summer Internship'")
            .execute(&db)
            .await
            .unwrap();
        let filters = ListingQuery {
            role: Some("intern".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &filters).await, 6);
    }

    #[tokio::test]
    async fn database_filters_match_canonical_tokens_locations_employment_and_salary_units() {
        let db = crate::test_database().await;
        let fixtures = [
            (
                "Software Development Engineer",
                "Bengaluru, India",
                Some("remote"),
                Some("FullTime"),
                Some("Product"),
                Some("Core"),
                Some(130000.0),
                Some(150000.0),
                Some("USD"),
                Some("year"),
            ),
            (
                "Front-end Engineer",
                "India",
                None,
                Some("full time"),
                Some("Product"),
                None,
                Some(50.0),
                Some(70.0),
                Some("USD"),
                Some("hour"),
            ),
            (
                "UI Designer",
                "Remote",
                Some("remote"),
                Some("Part_Time"),
                None,
                None,
                Some(90000.0),
                Some(90000.0),
                Some("USD"),
                Some("year"),
            ),
            (
                "Engineer",
                "Global office",
                Some("onsite"),
                Some("Full-time"),
                None,
                None,
                Some(100000.0),
                Some(110000.0),
                Some("USD"),
                Some("year"),
            ),
            (
                "Engineer",
                "Mumbai, India",
                None,
                Some("Intern"),
                None,
                None,
                None,
                None,
                None,
                None,
            ),
            (
                "Engineer",
                "India",
                Some("hybrid"),
                Some("Contractor"),
                None,
                None,
                Some(120000.0),
                Some(130000.0),
                Some("EUR"),
                Some("year"),
            ),
            (
                "Engineer",
                "Remote; Dublin",
                Some("remote"),
                None,
                None,
                None,
                Some(140000.0),
                Some(150000.0),
                Some("USD"),
                None,
            ),
            (
                "Discount 50%_off\\\\sale",
                "India",
                None,
                Some("Full Time"),
                Some("Sales"),
                None,
                None,
                None,
                None,
                None,
            ),
        ];
        for (
            index,
            (
                title,
                location,
                workplace,
                employment,
                department,
                team,
                low,
                high,
                currency,
                interval,
            ),
        ) in fixtures.into_iter().enumerate()
        {
            sqlx::query("INSERT INTO job_postings (source, source_job_id, title, company, location, workplace_type, employment_type, department, team, salary_min, salary_max, salary_currency, salary_interval, url) VALUES ('test', $1, $2, 'Fixture', $3, $4, $5, $6, $7, $8, $9, $10, $11, 'https://example.test/job')")
                .bind(index.to_string()).bind(title).bind(location).bind(workplace).bind(employment)
                .bind(department).bind(team).bind(low).bind(high).bind(currency).bind(interval)
                .execute(&db).await.unwrap();
        }

        let roles = ListingQuery {
            role: Some("software engineer".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &roles).await, 1);
        let front = ListingQuery {
            role: Some("front end".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &front).await, 1);
        let city = ListingQuery {
            location: Some("India".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &city).await, 5);
        let other_country_remote = ListingQuery {
            location: Some("Canada".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &other_country_remote).await, 0);
        let other_country_inclusive = ListingQuery {
            location: Some("Canada".into()),
            include_global: Some(true),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &other_country_inclusive).await, 2);
        let inclusive = ListingQuery {
            location: Some("India".into()),
            include_global: Some(true),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &inclusive).await, 7);
        let no_unknown = ListingQuery {
            workplace: Some("remote".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &no_unknown).await, 3);
        let with_unknown = ListingQuery {
            workplace: Some("remote".into()),
            include_unknown_workplace: Some(true),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &with_unknown).await, 6);
        let full_time = ListingQuery {
            employment: Some("full-time".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &full_time).await, 4);
        let part_time = ListingQuery {
            employment: Some("parttime".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &part_time).await, 1);
        let contract = ListingQuery {
            employment: Some("fixed-term".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &contract).await, 1);
        let internship = ListingQuery {
            employment: Some("Internship".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &internship).await, 1);
        let salary = ListingQuery {
            salary_min: Some(120000.0),
            currency: Some("USD".into()),
            salary_interval: Some("annual".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &salary).await, 1);
        let hourly = ListingQuery {
            salary_max: Some(100.0),
            currency: Some("USD".into()),
            salary_interval: Some("hour".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &hourly).await, 1);
        let literal = ListingQuery {
            role: Some("50%_off\\\\sale".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &literal).await, 1);
        let percent = ListingQuery {
            role: Some("%".into()),
            ..empty_filters()
        };
        assert_eq!(matching_count(&db, &percent).await, 1);
        for filters in [
            ListingQuery {
                location: Some("%".into()),
                ..empty_filters()
            },
            ListingQuery {
                workplace: Some("%".into()),
                ..empty_filters()
            },
            ListingQuery {
                department: Some("%".into()),
                ..empty_filters()
            },
            ListingQuery {
                currency: Some("%".into()),
                ..empty_filters()
            },
        ] {
            assert_eq!(matching_count(&db, &filters).await, 0);
        }
        let global = ListingQuery {
            location: Some("India".into()),
            include_global: Some(true),
            ..empty_filters()
        };
        // A broad substring such as "Global office" is deliberately not considered worldwide.
        let global_count = matching_count(&db, &global).await;
        assert_eq!(global_count, 7);
    }

    #[test]
    fn maps_ashby_compensation_only_from_salary_component_and_keeps_location_details() {
        let raw = serde_json::json!({
            "employmentType": "FullTime",
            "address": {"postalAddress": {"addressLocality": "Bengaluru", "addressCountry": "India"}},
            "secondaryLocations": [{"location": "Mumbai", "address": {"addressCountry": "India"}}],
            "compensation": {"summaryComponents": [
                {"compensationType": "EquityPercentage", "minValue": 1.0, "maxValue": 2.0},
                {"compensationType": "Salary", "interval": "1 YEAR", "currencyCode": "INR", "minValue": 1200000, "maxValue": 1800000}
            ]}
        });
        let job = JobPosting::basic(
            "ashby",
            "1",
            "Engineer".into(),
            "Example",
            Some("Remote".into()),
            None,
            "https://example.test/job".into(),
            &raw,
        );
        assert_eq!(job.employment_type.as_deref(), Some("Full-time"));
        assert_eq!(job.salary_min, Some(1_200_000.0));
        assert_eq!(job.salary_max, Some(1_800_000.0));
        assert_eq!(job.salary_currency.as_deref(), Some("INR"));
        assert_eq!(job.salary_interval.as_deref(), Some("year"));
        let api_job = serde_json::to_value(&job).unwrap();
        assert_eq!(api_job["salary_interval"], "year");
        let location = job.location.unwrap();
        assert!(
            location.contains("Remote")
                && location.contains("Bengaluru")
                && location.contains("India")
                && location.contains("Mumbai")
        );
    }

    #[test]
    fn maps_real_ats_date_salary_locations_and_categories() {
        let raw = serde_json::json!({
            "id": "native-42",
            "createdAt": 1751559111614i64,
            "salaryRange": {"min": 120000, "max": 160000, "currency": "USD", "interval": "year"},
            "workplaceType": "Hybrid",
            "secondaryLocations": [{"location": "Dublin"}],
            "categories": {"location": "Remote - UK", "allLocations": ["Remote - UK", "Dublin"], "commitment": "Full-time", "department": "Engineering", "team": "Platform"},
            "descriptionBody": "<p>Build systems</p>",
            "descriptionBodyPlain": "Build systems plainly"
        });
        let job = JobPosting::basic(
            "lever",
            "url-id",
            "Engineer".into(),
            "Example",
            None,
            None,
            "https://example.test/jobs/42".into(),
            &raw,
        );
        assert_eq!(job.posted_at.as_deref(), Some("2025-07-03T16:11:51.614Z"));
        assert_eq!(job.salary_min, Some(120000.0));
        assert_eq!(job.salary_max, Some(160000.0));
        assert_eq!(job.salary_currency.as_deref(), Some("USD"));
        assert_eq!(job.workplace_type.as_deref(), Some("hybrid"));
        assert_eq!(job.employment_type.as_deref(), Some("Full-time"));
        assert_eq!(job.department.as_deref(), Some("Engineering"));
        assert_eq!(job.team.as_deref(), Some("Platform"));
        assert_eq!(job.location.as_deref(), Some("Remote - UK; Dublin"));
        assert_eq!(
            job.description_text.as_deref(),
            Some("Build systems plainly")
        );
    }

    #[test]
    fn maps_greenhouse_first_published_and_office_metadata() {
        let raw = serde_json::json!({
            "first_published": "2026-10-02T11:31:50-04:00",
            "updated_at": "2026-10-02T12:00:00-04:00",
            "application_deadline": "2026-11-01",
            "departments": [{"name": "Product Engineering"}],
            "offices": [{"name": "Dublin", "location": "Ireland"}],
            "content": "<p>Role details</p>"
        });
        let job = JobPosting::basic(
            "greenhouse",
            "1",
            "Engineer".into(),
            "Example",
            None,
            None,
            "https://example.test/jobs/1".into(),
            &raw,
        );
        assert_eq!(job.posted_at.as_deref(), Some("2026-10-02T11:31:50-04:00"));
        assert_eq!(job.department.as_deref(), Some("Product Engineering"));
        assert_eq!(job.location.as_deref(), Some("Dublin, Ireland"));
        let mut adapter_override = job.clone();
        adapter_override.location = Some("Dublin".into());
        assert_eq!(
            location_for_storage(&adapter_override).as_deref(),
            Some("Dublin, Ireland")
        );
        assert_eq!(job.description.as_deref(), Some("<p>Role details</p>"));
    }

    #[test]
    fn preserves_raw_fields_and_normalizes_filterable_metadata() {
        let raw = serde_json::json!({
            "title": "Engineer",
            "publishedAt": "2026-10-01T09:30:00Z",
            "salary_min": "120000",
            "salary_max": 160000,
            "salaryCurrency": "USD",
            "workAuthorization": ["US", "Canada"],
            "customProviderField": {"keep": true}
        });
        let job = JobPosting::basic(
            "test",
            "42",
            "Engineer".into(),
            "Example",
            None,
            None,
            "https://example.test/jobs/42".into(),
            &raw,
        );
        assert_eq!(job.posted_at.as_deref(), Some("2026-10-01T09:30:00Z"));
        assert_eq!(job.salary_min, Some(120000.0));
        assert_eq!(job.salary_max, Some(160000.0));
        assert_eq!(job.salary_currency.as_deref(), Some("USD"));
        let details: serde_json::Value = serde_json::from_str(&job.details_json).unwrap();
        assert_eq!(details["workAuthorization"][0], "US");
        assert_eq!(details["customProviderField"]["keep"], true);
    }

    #[tokio::test]
    async fn exact_source_reconciliation_does_not_prefix_match_neighbor_sources() {
        let db = crate::test_database().await;
        sqlx::query("INSERT INTO job_postings (source, source_job_id, title, company, url) VALUES ('lever', 'old', 'Old', 'Example', 'https://example.test/old'), ('lever-v2', 'neighbor', 'Neighbor', 'Example', 'https://example.test/neighbor')")
            .execute(&db)
            .await
            .unwrap();

        let snapshot = vec![JobPosting::basic(
            "lever",
            "new",
            "New".into(),
            "Example",
            None,
            None,
            "https://example.test/new".into(),
            serde_json::json!({}),
        )];
        assert!(replace_source_snapshot(&db, "Example", "lever", &snapshot)
            .await
            .is_ok());

        let old_active: bool =
            sqlx::query_scalar("SELECT is_active FROM job_postings WHERE source_job_id = 'old'")
                .fetch_one(&db)
                .await
                .unwrap();
        let neighbor_active: bool = sqlx::query_scalar(
            "SELECT is_active FROM job_postings WHERE source_job_id = 'neighbor'",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        let new_active: bool =
            sqlx::query_scalar("SELECT is_active FROM job_postings WHERE source_job_id = 'new'")
                .fetch_one(&db)
                .await
                .unwrap();
        assert!(!old_active);
        assert!(neighbor_active);
        assert!(new_active);
    }

    #[tokio::test]
    async fn exact_source_reconciliation_rejects_mismatched_snapshot_before_deactivate() {
        let db = crate::test_database().await;
        sqlx::query("INSERT INTO job_postings (source, source_job_id, title, company, url) VALUES ('lever', 'old', 'Old', 'Example', 'https://example.test/old')")
            .execute(&db)
            .await
            .unwrap();

        let snapshot = vec![JobPosting::basic(
            "lever-v2",
            "new",
            "New".into(),
            "Example",
            None,
            None,
            "https://example.test/new".into(),
            serde_json::json!({}),
        )];
        assert!(replace_source_snapshot(&db, "Example", "lever", &snapshot)
            .await
            .is_err());

        let old_active: bool =
            sqlx::query_scalar("SELECT is_active FROM job_postings WHERE source_job_id = 'old'")
                .fetch_one(&db)
                .await
                .unwrap();
        assert!(old_active);
    }
}
