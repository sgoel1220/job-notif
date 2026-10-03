use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::errors::ApiError;

#[derive(Serialize, FromRow)]
pub(crate) struct Greeting {
    pub(crate) id: i64,
    pub(crate) name: String,
    pub(crate) created_at: String,
}

#[derive(Serialize, FromRow)]
pub(crate) struct BackgroundJob {
    pub(crate) id: i64,
    pub(crate) name: String,
    pub(crate) status: String,
    pub(crate) created_at: String,
}

#[derive(Deserialize)]
pub(crate) struct NewGreeting {
    pub(crate) name: String,
}

impl NewGreeting {
    pub(crate) fn validated_name(self) -> Result<String, ApiError> {
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > 80 {
            return Err(ApiError::bad_request("Name must be 1–80 characters."));
        }
        Ok(name.to_owned())
    }
}
