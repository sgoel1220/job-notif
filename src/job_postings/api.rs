use super::{
    filters::{append_listing_filters, validate_salary_filters},
    ListingPage, ListingQuery, StoredJobPosting, DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE,
};
use crate::{errors::ApiError, AppState};
use axum::{
    extract::{Query, State},
    Json,
};
use sqlx::{Postgres, QueryBuilder};

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
