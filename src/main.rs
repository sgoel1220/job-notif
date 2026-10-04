mod ats;
mod company_registry;
mod description_backfill;
mod errors;
mod job_postings;
mod listings;
mod remote;
mod signals;
mod state;
mod sync;
mod ui;

use axum::{routing::get, Router};
use sqlx::postgres::PgPoolOptions;
use state::AppState;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--backfill-descriptions") {
        dotenvy::dotenv().ok();
        let mut dry_run = false;
        let mut company = None;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--backfill-descriptions" => (),
                "--dry-run" => dry_run = true,
                "--company" => {
                    i += 1;
                    company = Some(args.get(i).ok_or("--company needs a value")?.as_str());
                }
                flag => return Err(format!("unknown backfill option: {flag}").into()),
            }
            i += 1;
        }
        let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let db = PgPoolOptions::new()
            .max_connections(3)
            .connect(&database_url)
            .await?;
        return description_backfill::run(&db, company, dry_run).await;
    }
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set to a PostgreSQL connection string");
    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await?;
    sqlx::migrate!().run(&db).await?;

    let state = AppState {
        db,
        sync_lock: Arc::new(tokio::sync::Mutex::new(())),
        manual_sync_last_request: Arc::new(std::sync::Mutex::new(None)),
    };
    let app = Router::new()
        .route("/", get(ui::home))
        .route("/api/listings", get(listings::list))
        .route("/api/job-postings", get(job_postings::list))
        .route("/api/sync", axum::routing::post(sync::sync_all))
        .with_state(state.clone());

    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "3000".to_owned())
        .parse::<u16>()?;
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tokio::spawn(sync::run_daily_sync(state));
    println!("Job Notifier listening on http://0.0.0.0:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
pub(crate) async fn test_database() -> sqlx::PgPool {
    sync::test_database().await
}
