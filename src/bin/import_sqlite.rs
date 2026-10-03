use sqlx::{postgres::PgPoolOptions, Postgres, QueryBuilder, Row, SqlitePool};
use std::{env, error::Error};

const BATCH: usize = 500;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenvy::dotenv().ok();
    let mut args = env::args().skip(1);
    let mut db_path = None;
    let mut force = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--force" => force = true,
            "--db" => db_path = args.next(),
            "-h" | "--help" => {
                println!("Usage: cargo run --features sqlite-import --bin import_sqlite -- --db <sqlite-file> [--force]\nDATABASE_URL must point to an already migrated PostgreSQL database.");
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    let db_path = db_path.ok_or("required argument --db <sqlite-file>")?;
    let url = env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is not set")?;
    let sqlite = SqlitePool::connect(&format!("sqlite://{}?mode=ro", db_path)).await?;
    let pg = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await?;
    sqlx::migrate!().run(&pg).await?;
    let mut tx = pg.begin().await?;
    sqlx::query(
        "LOCK TABLE greetings, background_jobs, sync_state, job_postings IN ACCESS EXCLUSIVE MODE",
    )
    .execute(&mut *tx)
    .await?;
    let mut occupied = Vec::new();
    for table in TABLES {
        let has_rows = if table == "sync_state" {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM sync_state WHERE id <> 1 OR last_successful_sync IS NOT NULL)",
            )
            .fetch_one(&mut *tx)
            .await?
        } else {
            sqlx::query_scalar::<_, bool>(&format!("SELECT EXISTS (SELECT 1 FROM {table})"))
                .fetch_one(&mut *tx)
                .await?
        };
        if has_rows {
            occupied.push(table);
        }
    }
    if !occupied.is_empty() && !force {
        return Err(format!(
            "Target has rows in {}. Refusing import; --force allows appending.",
            occupied.join(", ")
        )
        .into());
    }

    for rows in sqlx::query("SELECT id,name,created_at FROM greetings ORDER BY id")
        .fetch_all(&sqlite)
        .await?
        .chunks(BATCH)
    {
        let mut q = QueryBuilder::<Postgres>::new("INSERT INTO greetings (id,name,created_at) ");
        q.push_values(rows, |mut b, r| {
            b.push_bind(r.get::<i64, _>("id"))
                .push_bind(r.get::<String, _>("name"))
                .push_bind(r.get::<String, _>("created_at"));
        });
        q.build().execute(&mut *tx).await?;
    }
    for rows in sqlx::query("SELECT id,name,status,created_at FROM background_jobs ORDER BY id")
        .fetch_all(&sqlite)
        .await?
        .chunks(BATCH)
    {
        let mut q = QueryBuilder::<Postgres>::new(
            "INSERT INTO background_jobs (id,name,status,created_at) ",
        );
        q.push_values(rows, |mut b, r| {
            b.push_bind(r.get::<i64, _>("id"))
                .push_bind(r.get::<String, _>("name"))
                .push_bind(r.get::<String, _>("status"))
                .push_bind(r.get::<String, _>("created_at"));
        });
        q.build().execute(&mut *tx).await?;
    }
    for row in sqlx::query("SELECT id,last_successful_sync FROM sync_state ORDER BY id")
        .fetch_all(&sqlite)
        .await?
    {
        sqlx::query(
            "INSERT INTO sync_state (id,last_successful_sync) VALUES ($1, CAST($2 AS TIMESTAMPTZ))
             ON CONFLICT (id) DO UPDATE SET last_successful_sync = EXCLUDED.last_successful_sync",
        )
        .bind(row.get::<i32, _>("id"))
        .bind(row.get::<Option<String>, _>("last_successful_sync"))
        .execute(&mut *tx)
        .await?;
    }
    for rows in sqlx::query("SELECT id,source,source_job_id,title,company,location,workplace_type,employment_type,department,team,description,description_text,posted_at,salary_min,salary_max,salary_currency,url,details_json,first_seen_at,last_seen_at,is_active FROM job_postings ORDER BY id").fetch_all(&sqlite).await?.chunks(BATCH) {
        let mut q = QueryBuilder::<Postgres>::new("INSERT INTO job_postings (id,source,source_job_id,title,company,location,workplace_type,employment_type,department,team,description,description_text,posted_at,salary_min,salary_max,salary_currency,url,details_json,first_seen_at,last_seen_at,is_active) ");
        q.push_values(rows, |mut b, r| {
            b.push_bind(r.get::<i64,_>("id")).push_bind(r.get::<String,_>("source")).push_bind(r.get::<String,_>("source_job_id"))
             .push_bind(r.get::<String,_>("title")).push_bind(r.get::<String,_>("company"))
             .push_bind(r.get::<Option<String>,_>("location")).push_bind(r.get::<Option<String>,_>("workplace_type"))
             .push_bind(r.get::<Option<String>,_>("employment_type")).push_bind(r.get::<Option<String>,_>("department"))
             .push_bind(r.get::<Option<String>,_>("team")).push_bind(r.get::<Option<String>,_>("description"))
             .push_bind(r.get::<Option<String>,_>("description_text")).push_bind(r.get::<Option<String>,_>("posted_at"))
             .push_bind(r.get::<Option<f64>,_>("salary_min")).push_bind(r.get::<Option<f64>,_>("salary_max"))
             .push_bind(r.get::<Option<String>,_>("salary_currency")).push_bind(r.get::<String,_>("url"))
             .push_bind(r.get::<String,_>("details_json")).push_bind(r.get::<String,_>("first_seen_at"))
             .push_bind(r.get::<String,_>("last_seen_at")).push_bind(r.get::<bool,_>("is_active"));
        });
        q.build().execute(&mut *tx).await?;
    }
    for (table, col) in [
        ("greetings", "id"),
        ("background_jobs", "id"),
        ("job_postings", "id"),
    ] {
        sqlx::query(&format!("SELECT setval(pg_get_serial_sequence('{table}','{col}'), COALESCE((SELECT MAX({col}) FROM {table}), 1), EXISTS (SELECT 1 FROM {table}))")).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    println!("Import complete.");
    Ok(())
}

const TABLES: [&str; 4] = ["greetings", "background_jobs", "sync_state", "job_postings"];
