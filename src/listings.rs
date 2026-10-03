use axum::{extract::State, Json};
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

pub(crate) async fn list(State(state): State<AppState>) -> Result<Json<Vec<Listing>>, ApiError> {
    let listings = sqlx::query_as::<_, Listing>(
        "SELECT id, title, company, COALESCE(location, '') AS location, url
         FROM job_postings WHERE is_active = TRUE ORDER BY first_seen_at DESC, id DESC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(ApiError::database)?;
    Ok(Json(listings))
}
