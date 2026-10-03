use axum::{
    extract::{Query, State},
    Json,
};
use sqlx::FromRow;

use crate::{errors::ApiError, AppState};

#[derive(serde::Serialize, FromRow)]
pub(crate) struct Listing {
    pub(crate) id: i64,
    pub(crate) title: String,
    pub(crate) company: String,
    pub(crate) location: String,
    pub(crate) url: String,
}

#[derive(Default, serde::Deserialize)]
pub(crate) struct ListingQuery {
    page: Option<u64>,
    page_size: Option<u64>,
}

pub(crate) async fn list(
    State(state): State<AppState>,
    Query(filters): Query<ListingQuery>,
) -> Result<Json<Vec<Listing>>, ApiError> {
    let page = filters.page.unwrap_or(1).max(1);
    let page_size = filters
        .page_size
        .unwrap_or(crate::job_postings::DEFAULT_PAGE_SIZE)
        .clamp(1, crate::job_postings::MAX_PAGE_SIZE);
    let offset = (page - 1).saturating_mul(page_size).min(i64::MAX as u64);
    // Limit in PostgreSQL, not after fetching: never transfer the full snapshot to the app.
    let listings = sqlx::query_as::<_, Listing>(
        "SELECT id, title, company, COALESCE(location, '') AS location, url
         FROM job_postings WHERE is_active = TRUE
         ORDER BY first_seen_at DESC, id DESC LIMIT $1 OFFSET $2",
    )
    .bind(page_size as i64)
    .bind(offset as i64)
    .fetch_all(&state.db)
    .await
    .map_err(ApiError::database)?;
    Ok(Json(listings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // Convert the API error to a debug-printable tuple for clear test failures.
    async fn list(
        state: State<AppState>,
        query: Query<ListingQuery>,
    ) -> Result<Json<Vec<Listing>>, (axum::http::StatusCode, String)> {
        super::list(state, query)
            .await
            .map_err(|error| (error.0, error.1))
    }

    async fn fixture() -> AppState {
        let db = crate::test_database().await;
        sqlx::query(
            "INSERT INTO job_postings (source, source_job_id, title, company, url, first_seen_at)
             SELECT 'fixture', n::text, 'Job ' || n, 'Example', 'https://example.test/' || n,
                    '2026-10-03T00:00:00Z'
             FROM generate_series(1, 130) AS n",
        )
        .execute(&db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO job_postings (source, source_job_id, title, company, url, is_active, first_seen_at)
             VALUES ('fixture', 'inactive', 'Inactive', 'Example', 'https://example.test/inactive', FALSE,
                     '2099-01-01T00:00:00Z')",
        ).execute(&db).await.unwrap();
        AppState {
            db,
            sync_lock: Arc::new(tokio::sync::Mutex::new(())),
            manual_sync_last_request: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    #[tokio::test]
    async fn default_page_is_bounded_active_and_preserves_array_response() {
        let state = fixture().await;
        let Json(jobs) = list(State(state.clone()), Query(ListingQuery::default()))
            .await
            .unwrap();
        assert_eq!(jobs.len(), 25);
        assert_eq!(jobs[0].title, "Job 130");
        assert_eq!(jobs[24].title, "Job 106");
        assert!(jobs
            .iter()
            .all(|job| job.title != "Inactive" && job.location.is_empty()));
        assert!(serde_json::to_value(&jobs).unwrap().is_array());
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM job_postings WHERE is_active = TRUE")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(count, 130);
    }

    #[tokio::test]
    async fn pages_are_disjoint_stably_ordered_and_end_with_empty_array() {
        let state = fixture().await;
        let Json(first) = list(
            State(state.clone()),
            Query(ListingQuery {
                page: Some(1),
                page_size: Some(50),
            }),
        )
        .await
        .unwrap();
        let Json(second) = list(
            State(state.clone()),
            Query(ListingQuery {
                page: Some(2),
                page_size: Some(50),
            }),
        )
        .await
        .unwrap();
        let Json(last) = list(
            State(state.clone()),
            Query(ListingQuery {
                page: Some(3),
                page_size: Some(50),
            }),
        )
        .await
        .unwrap();
        let Json(empty) = list(
            State(state),
            Query(ListingQuery {
                page: Some(4),
                page_size: Some(50),
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            (first.len(), second.len(), last.len(), empty.len()),
            (50, 50, 30, 0)
        );
        assert_eq!(second[0].title, "Job 80");
        assert_eq!(last[29].title, "Job 1");
        let ids = first
            .iter()
            .chain(&second)
            .chain(&last)
            .map(|job| job.id)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(ids.len(), 130);
    }

    #[tokio::test]
    async fn pagination_inputs_are_clamped_without_overflow() {
        let state = fixture().await;
        let Json(maximum) = list(
            State(state.clone()),
            Query(ListingQuery {
                page: Some(0),
                page_size: Some(u64::MAX),
            }),
        )
        .await
        .unwrap();
        assert_eq!(maximum.len(), 100);
        assert_eq!(maximum[0].title, "Job 130");
        let Json(minimum) = list(
            State(state.clone()),
            Query(ListingQuery {
                page: Some(0),
                page_size: Some(0),
            }),
        )
        .await
        .unwrap();
        assert_eq!(minimum.len(), 1);
        let Json(empty) = list(
            State(state),
            Query(ListingQuery {
                page: Some(u64::MAX),
                page_size: Some(u64::MAX),
            }),
        )
        .await
        .unwrap();
        assert!(empty.is_empty());
    }
}
