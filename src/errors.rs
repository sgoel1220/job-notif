use axum::{http::StatusCode, response::IntoResponse};

pub(crate) struct ApiError(pub(crate) StatusCode, pub(crate) String);

impl ApiError {
    pub(crate) fn bad_request(message: &str) -> Self {
        Self(StatusCode::BAD_REQUEST, message.to_owned())
    }

    pub(crate) fn database(error: sqlx::Error) -> Self {
        eprintln!("Database error: {error}");
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Database error.".to_owned(),
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.0, self.1).into_response()
    }
}
