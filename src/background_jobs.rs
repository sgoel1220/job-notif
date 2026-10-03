use axum::{extract::State, http::StatusCode, Json};
use sqlx::{query, query_as, PgPool};
use tokio::time::{sleep, Duration};

use crate::{
    errors::ApiError,
    models::{BackgroundJob, NewGreeting},
    AppState,
};

pub(crate) async fn list(
    State(state): State<AppState>,
) -> Result<Json<Vec<BackgroundJob>>, ApiError> {
    let jobs = query_as::<_, BackgroundJob>(
        "SELECT id, name, status, created_at FROM background_jobs ORDER BY id DESC LIMIT 50",
    )
    .fetch_all(&state.db)
    .await
    .map_err(ApiError::database)?;
    Ok(Json(jobs))
}

pub(crate) async fn enqueue(
    State(state): State<AppState>,
    Json(input): Json<NewGreeting>,
) -> Result<(StatusCode, Json<BackgroundJob>), ApiError> {
    let name = input.validated_name()?;
    let job = query_as::<_, BackgroundJob>(
        "INSERT INTO background_jobs (name) VALUES ($1)
         RETURNING id, name, status, created_at",
    )
    .bind(name)
    .fetch_one(&state.db)
    .await
    .map_err(ApiError::database)?;
    Ok((StatusCode::ACCEPTED, Json(job)))
}

/// Poll the durable PostgreSQL queue and turn each request into a saved greeting.
pub(crate) async fn run_worker(db: PgPool) {
    loop {
        let next_job = query_as::<_, BackgroundJob>(
            "UPDATE background_jobs SET status = 'running' WHERE id = (
                SELECT id FROM background_jobs WHERE status = 'queued' ORDER BY id LIMIT 1 FOR UPDATE SKIP LOCKED
            ) RETURNING id, name, status, created_at",
        )
        .fetch_optional(&db)
        .await;

        match next_job {
            Ok(Some(job)) => process_job(&db, job).await,
            Ok(None) => sleep(Duration::from_secs(1)).await,
            Err(error) => {
                eprintln!("Background worker database error: {error}");
                sleep(Duration::from_secs(3)).await;
            }
        }
    }
}

async fn process_job(db: &PgPool, job: BackgroundJob) {
    println!("Processing background job #{} for {}", job.id, job.name);
    sleep(Duration::from_secs(3)).await; // Simulate slow/offline work.

    let result = async {
        let mut tx = db.begin().await?;
        query("INSERT INTO greetings (name) VALUES ($1)")
            .bind(&job.name)
            .execute(&mut *tx)
            .await?;
        query("UPDATE background_jobs SET status = 'completed' WHERE id = $1")
            .bind(job.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await
    }
    .await;

    match result {
        Ok(()) => println!("Completed background job #{}", job.id),
        Err(error) => {
            eprintln!("Background job #{} failed: {error}", job.id);
            let _ = query("UPDATE background_jobs SET status = 'queued' WHERE id = $1")
                .bind(job.id)
                .execute(db)
                .await;
        }
    }
}
