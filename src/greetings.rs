use axum::{extract::State, http::StatusCode, Json};
use sqlx::query_as;

use crate::{
    errors::ApiError,
    models::{Greeting, NewGreeting},
    AppState,
};

pub(crate) async fn list(State(state): State<AppState>) -> Result<Json<Vec<Greeting>>, ApiError> {
    let greetings =
        query_as::<_, Greeting>("SELECT id, name, created_at FROM greetings ORDER BY id DESC")
            .fetch_all(&state.db)
            .await
            .map_err(ApiError::database)?;
    Ok(Json(greetings))
}

pub(crate) async fn create(
    State(state): State<AppState>,
    Json(input): Json<NewGreeting>,
) -> Result<(StatusCode, Json<Greeting>), ApiError> {
    let name = input.validated_name()?;
    let greeting = query_as::<_, Greeting>(
        "INSERT INTO greetings (name) VALUES ($1) RETURNING id, name, created_at",
    )
    .bind(name)
    .fetch_one(&state.db)
    .await
    .map_err(ApiError::database)?;
    Ok((StatusCode::CREATED, Json(greeting)))
}
