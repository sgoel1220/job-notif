mod manual;
mod runner;
mod scheduler;

pub(crate) use manual::sync_all;
pub(crate) use scheduler::run_daily_sync;

#[cfg(test)]
pub(crate) async fn test_database() -> sqlx::PgPool {
    tests::test_database().await
}

#[cfg(test)]
mod tests;
