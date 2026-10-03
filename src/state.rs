use sqlx::PgPool;
use std::{sync::Arc, time::Instant};

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) db: PgPool,
    pub(crate) sync_lock: Arc<tokio::sync::Mutex<()>>,
    pub(crate) manual_sync_last_request: Arc<std::sync::Mutex<Option<Instant>>>,
}
