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
        job.employment_type = first_string(
            &details_value,
            &["employmentType", "employment_type", "employmentStatus"],
        )
        .or_else(|| nested_string(&details_value, &["categories", "commitment"]));
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
    for key in ["locations", "secondaryLocations", "offices"] {
        if let Some(items) = value.get(key).and_then(serde_json::Value::as_array) {
            for item in items {
                let text = value_to_string(item)
                    .or_else(|| item.get("name").and_then(value_to_string))
                    .or_else(|| item.get("location").and_then(value_to_string))
                    .or_else(|| {
                        item.pointer("/address/postalAddress")
                            .and_then(value_to_string)
                    });
                if let Some(text) = text {
                    all.push(text);
                }
            }
        }
    }
    if let Some(items) = value
        .pointer("/categories/allLocations")
        .and_then(serde_json::Value::as_array)
    {
        all.extend(items.iter().filter_map(value_to_string));
    }
    let mut seen = std::collections::HashSet::new();
    all.retain(|item| seen.insert(item.to_lowercase()));
    (!all.is_empty()).then(|| all.join("; "))
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
                salary_currency, url, details_json, is_active
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, TRUE)
             ON CONFLICT(company, source, source_job_id) DO UPDATE SET
                title = excluded.title, location = excluded.location,
                workplace_type = excluded.workplace_type, employment_type = excluded.employment_type,
                department = excluded.department, team = excluded.team,
                description = excluded.description, description_text = excluded.description_text,
                posted_at = excluded.posted_at, salary_min = excluded.salary_min,
                salary_max = excluded.salary_max, salary_currency = excluded.salary_currency,
                url = excluded.url, details_json = excluded.details_json,
                last_seen_at = CURRENT_TIMESTAMP::text, is_active = TRUE"
        )
        .bind(&job.source).bind(&job.source_job_id).bind(&job.title).bind(&job.company)
        .bind(&job.location).bind(&job.workplace_type).bind(&job.employment_type)
        .bind(&job.department).bind(&job.team).bind(&job.description).bind(&job.description_text)
        .bind(&job.posted_at).bind(job.salary_min).bind(job.salary_max).bind(&job.salary_currency)
        .bind(&job.url).bind(&job.details_json)
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
                salary_currency, url, details_json, is_active
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, TRUE)
             ON CONFLICT(company, source, source_job_id) DO UPDATE SET
                title = excluded.title, location = excluded.location,
                workplace_type = excluded.workplace_type, employment_type = excluded.employment_type,
                department = excluded.department, team = excluded.team,
                description = excluded.description, description_text = excluded.description_text,
                posted_at = excluded.posted_at, salary_min = excluded.salary_min,
                salary_max = excluded.salary_max, salary_currency = excluded.salary_currency,
                url = excluded.url, details_json = excluded.details_json,
                last_seen_at = CURRENT_TIMESTAMP::text, is_active = TRUE",
        )
        .bind(&job.source)
        .bind(&job.source_job_id)
        .bind(&job.title)
        .bind(&job.company)
        .bind(&job.location)
        .bind(&job.workplace_type)
        .bind(&job.employment_type)
        .bind(&job.department)
        .bind(&job.team)
        .bind(&job.description)
        .bind(&job.description_text)
        .bind(&job.posted_at)
        .bind(job.salary_min)
        .bind(job.salary_max)
        .bind(&job.salary_currency)
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
    pub(crate) url: String,
    pub(crate) is_active: bool,
}

#[derive(Deserialize)]
pub(crate) struct ListingQuery {
    page: Option<u64>,
    page_size: Option<u64>,
    role: Option<String>,
    location: Option<String>,
    workplace: Option<String>,
    employment: Option<String>,
    department: Option<String>,
    posted_after: Option<String>,
    posted_before: Option<String>,
    salary_min: Option<f64>,
    salary_max: Option<f64>,
    currency: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct ListingPage {
    jobs: Vec<StoredJobPosting>,
    total: i64,
    page: u64,
    page_size: u64,
}

const DEFAULT_PAGE_SIZE: u64 = 25;
const MAX_PAGE_SIZE: u64 = 100;

fn append_listing_filters<'a>(query: &mut QueryBuilder<'a, Postgres>, filters: &'a ListingQuery) {
    query.push(" WHERE is_active = TRUE");
    if let Some(value) = filters
        .role
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let pattern = format!("%{}%", value.trim());
        query.push(" AND title ILIKE ").push_bind(pattern);
    }
    if let Some(value) = filters
        .location
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        query
            .push(" AND COALESCE(location, '') ILIKE ")
            .push_bind(format!("%{}%", value.trim()));
    }
    if let Some(value) = filters
        .workplace
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        query
            .push(" AND COALESCE(workplace_type, '') ILIKE ")
            .push_bind(format!("%{}%", value.trim()));
    }
    if let Some(value) = filters
        .employment
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        query
            .push(" AND COALESCE(employment_type, '') ILIKE ")
            .push_bind(format!("%{}%", value.trim()));
    }
    if let Some(value) = filters
        .department
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let pattern = format!("%{}%", value.trim());
        query
            .push(" AND (COALESCE(department, '') ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR COALESCE(team, '') ILIKE ")
            .push_bind(pattern)
            .push(")");
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
        query
            .push(" AND COALESCE(salary_currency, '') ILIKE ")
            .push_bind(format!("%{}%", value.trim()));
    }
}

pub(crate) async fn list(
    State(state): State<AppState>,
    Query(filters): Query<ListingQuery>,
) -> Result<Json<ListingPage>, ApiError> {
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
         salary_max, salary_currency, url, is_active FROM job_postings",
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

    #[test]
    fn role_filter_searches_role_metadata_not_description_or_company() {
        let filters = ListingQuery {
            page: None,
            page_size: None,
            role: Some("intern".into()),
            location: None,
            workplace: None,
            employment: None,
            department: None,
            posted_after: None,
            posted_before: None,
            salary_min: None,
            salary_max: None,
            currency: None,
        };
        let mut query = QueryBuilder::<Postgres>::new("SELECT * FROM job_postings");
        append_listing_filters(&mut query, &filters);
        let sql = query.build().sql().to_owned();
        assert!(sql.contains("title ILIKE"));
        assert!(!sql.contains("department"));
        assert!(!sql.contains("team"));
        assert!(!sql.contains("description"));
        assert!(!sql.contains("company ILIKE"));
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
        assert_eq!(job.location.as_deref(), Some("Dublin"));
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
