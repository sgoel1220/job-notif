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

#[path = "tests/categories.rs"]
mod categories;
#[path = "tests/filters.rs"]
mod filters;
#[path = "tests/normalization.rs"]
mod normalization;
#[path = "tests/persistence.rs"]
mod persistence;
