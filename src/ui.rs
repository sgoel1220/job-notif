use askama::Template;
use axum::{http::StatusCode, response::Html};

use crate::errors::ApiError;

#[derive(Template)]
#[template(path = "jobs.html")]
struct JobsPage;

pub(crate) async fn home() -> Result<Html<String>, ApiError> {
    let html = JobsPage.render().map_err(|error| {
        eprintln!("Template render error: {error}");
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not render page.".into(),
        )
    })?;
    Ok(Html(html))
}
